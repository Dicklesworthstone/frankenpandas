# br-frankenpandas-8qbdv - parallel value_counts output materialization

LavenderStone, 2026-06-13.

## Target

Profile-backed bead: `br-frankenpandas-8qbdv`, opened after the kept
`uza04.103` Float64 bit-tally fast path. The cited residual is
`fp-bench --category dataframe_ops --workload value_counts --size 100k --dtype float64`:
100k all-valid Float64 distinct values still spend a large share of time in
post-tally output materialization, especially `format!("{v:?}")` in
`scalar_to_value_counts_index_label` plus count `Scalar` construction.

Alien-graveyard primitive used: deterministic morsel/chunk parallelism over an
embarrassingly independent output stage. This keeps row/order semantics fixed by
joining chunks in input order.

## Baseline

Build:

- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavenderstone-8qbdv-base rch exec -- cargo build -p fp-bench --profile release-perf --bin fp-bench`
- Remote worker: `vmi1153651`
- Artifact: `tests/artifacts/perf/8qbdv_base_build_fp_bench.txt`

Direct `fp-bench` timing, 25 in-process iterations:

- Artifact: `tests/artifacts/perf/8qbdv_base_fp_bench_value_counts_100k_float64.json`
- sha256: `d95d1545dba26758f1d7ab43cf2337b5b6362086f074eb79a2c07436439054b4`
- min: `12801.220 us`
- p50: `15449.039 us`
- mean: `15978.990 us`
- max: `25096.124 us`

Hyperfine full-process wrapper, 10 runs:

- Artifact: `tests/artifacts/perf/8qbdv_base_hyperfine_fp_bench_value_counts_100k_float64.json`
- sha256: `842dd23d1fa6e5e9a93060866aae0ccc0380dcb18d5682d96f7a16a6d6847bd7`
- mean: `439.419 ms +/- 34.428 ms`

Golden manifest before:

- Artifact: `tests/artifacts/perf/8qbdv_base_value_counts_goldens.sha256`
- sha256: `ac3922304b9caa5604ae14a9f95baaa00a41c838be26b2d3fda84e04ceb3fa68`
- `series_value_counts_basic.txt`: `8cedbe33b7c286e9ef7998bee56da5c402124ee06cd47e88f78b27ff2be5de9b`
- `value_counts_basic.txt`: `5122667d6f69c7f7414a38e8c937a325db2a9baacca73af8000438e61fbcd4c6`
- `dataframe_value_counts_basic.txt`: `ca68b11d3ce01363b58c9c98919d1a68bffed7a9aa82712ff998976a9e679bb3`

## Lever

One lever: after the existing first-seen count vector has been stably sorted by
descending count, split only the final materialization pass across up to 8 scoped
threads when `counts.len() >= 16384`.

Serial code:

- Iterate sorted `(Scalar, usize)` values.
- Push `scalar_to_value_counts_index_label(&value)`.
- Push `Scalar::Int64(i64::try_from(count).unwrap_or(i64::MAX))`.

New code:

- Uses the exact same mapping in each disjoint chunk.
- Joins chunk results in original chunk order.
- Keeps the serial path for small outputs.

## Isomorphism proof

- Ordering and ties: unchanged. The existing stable descending sort still runs
  before materialization. The helper partitions the already-sorted vector into
  contiguous chunks and concatenates chunk outputs in spawn order, which is the
  original vector order.
- Labels: unchanged. Every item still calls the same
  `scalar_to_value_counts_index_label` function with the same `Scalar`.
  Float64 labels still use the current `Utf8(format!("{v:?}"))` behavior.
- Counts: unchanged. Every item still emits
  `Scalar::Int64(i64::try_from(count).unwrap_or(i64::MAX))`.
- Floating point: no new floating-point arithmetic, comparisons, NaN handling,
  or bit canonicalization. The Float64 tally path and stable sort are untouched.
- Hashing: no new hash table and no changed hash order. Hash/tally code is
  untouched.
- RNG: none.
- Errors: only new failure path is worker panic mapped to
  `FrameError::CompatibilityRejected`; successful non-panicking executions map
  identically to the serial code.

## After

Final direct `fp-bench` confirmation, 25 in-process iterations:

- Artifact: `tests/artifacts/perf/8qbdv_after_final_confirm_fp_bench_value_counts_100k_float64.json`
- min: `11586.484 us`
- p50: `12552.955 us`
- mean: `13028.393 us`
- max: `16104.902 us`
- p50 speedup vs baseline: `1.2307x`
- mean speedup vs baseline: `1.2265x`

Final hyperfine full-process wrapper, 10 runs:

- Artifact: `tests/artifacts/perf/8qbdv_after_final_hyperfine_fp_bench_value_counts_100k_float64.json`
- sha256: `305bbaf14cc4bf9fa69da657cfe488bc42a41b0aa7dc67a22ac5c3b717da9d44`
- mean: `367.904 ms +/- 10.694 ms`
- speedup vs baseline mean: `1.1944x`

Final golden manifest after:

- Artifact: `tests/artifacts/perf/8qbdv_after_final_value_counts_goldens.sha256`
- sha256: `ac3922304b9caa5604ae14a9f95baaa00a41c838be26b2d3fda84e04ceb3fa68`
- Manifest is byte-identical to baseline.

Score: `Impact 3.0 x Confidence 0.85 / Effort 1.0 = 2.55`, keep.

## Validation

- `rch exec -- cargo check -p fp-frame --lib`
  - Artifact: `tests/artifacts/perf/8qbdv_after_check_fp_frame_lib.txt`
  - Result: pass. RCH fell open locally due `no admissible workers:
    insufficient_slots=2,hard_preflight=10`.
- `rch exec -- cargo test -p fp-frame value_counts --lib -- --nocapture`
  - Artifact: `tests/artifacts/perf/8qbdv_after_final_test_fp_frame_value_counts.txt`
  - Result: `32 passed; 0 failed; 1 ignored`.
  - RCH fell open locally due the same preflight pressure.
- `rch exec -- cargo clippy -p fp-frame --lib -- -D warnings`
  - Artifact: `tests/artifacts/perf/8qbdv_after_clippy_fp_frame_lib_rerun.txt`
  - Result: pass.
  - RCH fell open locally due the same preflight pressure.
- `cargo fmt -p fp-frame -- --check`
  - Artifact: `tests/artifacts/perf/8qbdv_cargo_fmt_fp_frame_check.txt`
  - Result: command exited 0 but printed pre-existing unrelated formatting diffs
    around prior sort/take/dot/reindex changes. The new value_counts helper is
    not in the fmt diff. Formatting cleanup intentionally not co-landed.
- `git diff --check`
  - Result: pass.
- `ubs crates/fp-frame/src/lib.rs`
  - Artifact: `tests/artifacts/perf/8qbdv_ubs_fp_frame_lib.txt`
  - Result: inconclusive; process ended without findings or result footer after
    printing the startup banner and `Scanning rust...`.
