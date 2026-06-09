# br-frankenpandas-jbyuc.1.1.1.1.1.1.1.1.1.1.1 Proof

## Change

Profile-backed lever: the ordered UTF8 join residual gate now computes its
numeric checksum through typed column slices when the merged output columns are
already all-valid `Float64` or `Int64`. Nullable, mixed, or otherwise untyped
columns fall back to the previous `Column::values()` plus `Scalar::to_f64()`
path.

This is a measurement/output-inspection lever for the setup-free join gate. It
does not alter production merge semantics. The profile after
`br-frankenpandas-jbyuc.1.1.1.1.1.1.1.1.1.1` showed the production ordered
UTF8 inner merge itself had reached the gate's timing floor; the remaining
full-command cost was scalar materialization in the checksum.

## Baseline

- Head: `8418a8ab11b1fb6d7f72e4b693795c50914439ac`.
- Build: `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-jbyuc11111111111-gate-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-join --profile release-perf --bin join-bench`.
- Gate command: `join-bench --str-ordered --rows 1000000 --right-rows 1000000 --join-type inner --warmup 3 --iters 20`.
- Baseline hyperfine: `125.5 ms +/- 3.6 ms`.
- Baseline internal timing with 50 iterations: `mean_ms=0.001`, `p50_ms=0.001`, `p95_ms=0.002`, `p99_ms=0.004`.
- Baseline golden SHA: `2ac49173153820d4b3878817c44be31979faa18b2ae034167f7977adee83b02e`.

## Isomorphism

- Ordering preserved: yes. The merge call and output rows are unchanged.
- Tie-breaking preserved: yes. The checksum walks the same left output column,
  then the same right output column, in row order.
- Floating-point behavior preserved: yes for the targeted all-valid `Float64`
  output. `as_f64_slice()` exposes the same raw values that
  `Scalar::Float64(value).to_f64()` returned; accumulation order is unchanged.
- Integer behavior preserved: yes for all-valid `Int64`; each value is cast to
  `f64` in the same row order as `Scalar::Int64(value).to_f64()`.
- Null/NaN behavior preserved: yes. Typed fast paths are only used for
  all-valid typed slices; nullable or mixed columns continue through the old
  scalar path.
- RNG unchanged: yes. No RNG is involved.
- Golden output unchanged: before and after gate goldens are byte-identical.

## Bench

- Paired A/B: baseline `155.5 ms +/- 15.8 ms`; after `138.7 ms +/- 10.0 ms`;
  after ran `1.12x +/- 0.14` faster.
- Reversed A/B: after `106.8 ms +/- 2.8 ms`; baseline `145.2 ms +/- 25.6 ms`;
  after ran `1.36x +/- 0.24` faster.
- Internal gate timing with 50 iterations: baseline `p95_ms=0.002`,
  `p99_ms=0.004`; after `p95_ms=0.001`, `p99_ms=0.001`.
- Score: Impact 2.5 x Confidence 3 / Effort 1.5 = 5.0. Keep.

## Shifted Profile

The after profile no longer shows `Scalar::to_f64` in the visible hot path.
The full command shifts to setup and first-use certificate work:

- `join_bench::build_ordered_utf8_frame`: `71.53%`.
- `join_bench::merge_once` / lower-hex certificate initialization: `22.19%`.
- `drop_in_place::<fp_frame::DataFrame>`: `6.12%`.

Next route should attack setup/certificate reuse or a production gate that
isolates repeated merge timing without scalarizing the output.

## Validation

- `cargo fmt -p fp-join -- --check`.
- `cargo check -p fp-join --all-targets`.
- `cargo clippy -p fp-join --all-targets -- -D warnings`.
- `ubs crates/fp-join/src/bin/join-bench.rs`: no critical issues; warnings are
  pre-existing bench-file surfaces or broad inventory.

## Artifacts

- `tests/artifacts/perf/golden_join_bench_base_jbyuc11111111111_str_ordered_1m.sha256`
- `tests/artifacts/perf/golden_join_bench_after_jbyuc11111111111_str_ordered_1m.sha256`
- `tests/artifacts/perf/golden_join_bench_compare_jbyuc11111111111.txt`
- `tests/artifacts/perf/hyperfine_join_bench_base_jbyuc11111111111_str_ordered_1m.txt`
- `tests/artifacts/perf/hyperfine_join_bench_pair_jbyuc11111111111_str_ordered_1m.txt`
- `tests/artifacts/perf/hyperfine_join_bench_pair_reversed_jbyuc11111111111_str_ordered_1m.txt`
- `tests/artifacts/perf/join_bench_base_jbyuc11111111111_str_ordered_1m_i50.txt`
- `tests/artifacts/perf/join_bench_after_jbyuc11111111111_str_ordered_1m_i50.txt`
- `tests/artifacts/perf/perf_join_bench_report_after_jbyuc11111111111_str_ordered_1m.txt`
