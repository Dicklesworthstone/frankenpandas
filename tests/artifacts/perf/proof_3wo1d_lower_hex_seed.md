# br-frankenpandas-3wo1d lower-hex certificate seed proof

## Target

- Bead: `br-frankenpandas-3wo1d`
- Profile-backed hotspot: full `join-bench --str-ordered` on ordered UTF8 inner joins.
- Residual before this lever: benchmark setup and first-use lower-hex certificate initialization after the prior typed-checksum gate.
- Alien primitive: producer-carried semantic witness / cache metadata reuse. Generated ordered lower-hex keys carry the construction certificate instead of rediscovering it with a full row scan at first merge.

## Lever

One lever only:

- Add `Column::from_lower_hex_sequence_utf8(prefix, start, len, hex_width)`.
- It emits the same contiguous UTF8 bytes as the old benchmark fixture for the fixed-width domain.
- It seeds immutable `strictly_increasing`, `fixed_width`, and `Utf8LowerHexSequence` witnesses at construction.
- `join-bench --str-ordered` uses the constructor when valid and falls back to the prior byte builder outside the fixed-width certificate domain.

## Behavior Isomorphism

- Ordering: unchanged. For fixed-width lowercase hex with a shared non-empty prefix, row `i` is exactly `prefix + hex(start + i)`, so lexicographic order equals numeric order. Invalid or non-fixed-width shapes fall back to the old builder.
- Tie-breaking: unchanged. Keys remain unique for the measured ordered fixture, and merge pair ordering still comes from the existing ordered-join plan.
- Floating point: unchanged. Payload generation is the same expression as before; only the key column constructor changed.
- RNG: none.
- Error/fallback behavior: empty fixtures still produce empty UTF8 columns with no lower-hex certificate; out-of-width shapes use the old contiguous UTF8 path.

## Golden SHA

- Main workload before: `2ac49173153820d4b3878817c44be31979faa18b2ae034167f7977adee83b02e`
- Main workload after: `2ac49173153820d4b3878817c44be31979faa18b2ae034167f7977adee83b02e`
- Empty ordered fixture before: `fc03a4635d1fe035e39a6f625acc9a3093dae0e9c61429a5a5c9742b146d0129`
- Empty ordered fixture after: `fc03a4635d1fe035e39a6f625acc9a3093dae0e9c61429a5a5c9742b146d0129`

Commands:

```bash
/data/projects/.scratch/cargo-target-orangepeak-3wo1d-f8e9-before/release-perf/join-bench --str-ordered --join-type inner --rows 1000000 --right-rows 1000000 --iters 1 --warmup 0 --golden | sha256sum
/data/projects/.scratch/cargo-target-orangepeak-3wo1d-f8e9-after/release-perf/join-bench --str-ordered --join-type inner --rows 1000000 --right-rows 1000000 --iters 1 --warmup 0 --golden | sha256sum
/data/projects/.scratch/cargo-target-orangepeak-3wo1d-f8e9-before/release-perf/join-bench --str-ordered --join-type inner --rows 0 --right-rows 0 --iters 1 --warmup 0 --golden | sha256sum
/data/projects/.scratch/cargo-target-orangepeak-3wo1d-f8e9-after/release-perf/join-bench --str-ordered --join-type inner --rows 0 --right-rows 0 --iters 1 --warmup 0 --golden | sha256sum
```

## Benchmark Gate

Current-main baseline, same-command A/B:

- Before: `98.4 ms +/- 3.5 ms`
- After: `78.8 ms +/- 4.8 ms`
- Speedup: `1.25x +/- 0.09`

Initial single-command baseline artifact:

- Before-only: `99.0 ms +/- 3.9 ms`

Raw `join-bench` checksums:

- Before checksum: `4999706700.000`
- After checksum: `4999706700.000`

Score:

- Impact `3`
- Confidence `4`
- Effort `2`
- Score `3 * 4 / 2 = 6.0`, keep.

Artifacts:

- `tests/artifacts/perf/hyperfine_3wo1d_before.json`
- `tests/artifacts/perf/hyperfine_3wo1d_before_after.json`
- `tests/artifacts/perf/join_bench_3wo1d_before.txt`
- `tests/artifacts/perf/join_bench_3wo1d_after.txt`

## Validation

```bash
env CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-3wo1d-f8e9-before rch exec -- cargo build -p fp-join --profile release-perf --bin join-bench
env CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-3wo1d-f8e9-after rch exec -- cargo build -p fp-join --profile release-perf --bin join-bench
env CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-3wo1d-final-verify rch exec -- cargo test -p fp-columnar lower_hex_sequence --lib -- --nocapture
env CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-3wo1d-final-verify rch exec -- cargo test -p fp-join ordered_unique_utf8_lower_hex --lib -- --nocapture
env CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-3wo1d-final-verify rch exec -- cargo check -p fp-columnar -p fp-join --all-targets
env CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-3wo1d-final-verify rch exec -- cargo clippy -p fp-columnar -p fp-join --all-targets -- -D warnings
cargo fmt -p fp-columnar -p fp-join -- --check
ubs crates/fp-columnar/src/lib.rs crates/fp-join/src/bin/join-bench.rs
```

Notes:

- RCH failed open to local execution, but every compile/test command stayed crate-scoped.
- UBS exited `0`; it reported broad file-wide warning inventories and clean build-health/clippy sections.

## Re-profile

Post-change perf report:

- `tests/artifacts/perf/perf_report_after_3wo1d_lower_hex_seed.txt`

Visible shifted residual:

- `Column::from_lower_hex_sequence_utf8` is now the top visible setup cost.
- Next primitive target: generate the fixed-width lower-hex byte buffer with a deeper structural producer, such as chunked/SWAR hex emission or reusable fixture buffers, while keeping the same constructor witness and golden SHA.
