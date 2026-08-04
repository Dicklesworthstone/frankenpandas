# br-frankenpandas-uza04.81 proof: quarter-affine Float64 CSV formatter

Date: 2026-06-11
Agent: OrangePeak
Head before proof: 217618ecfd25

## Target

The refreshed high-ram profile for `csv_write_read_roundtrip` still showed
`ryu::pretty::format64` as the dominant CSV Float64 residual after the generic
formatter removal from `.80`.

## Lever

Added a private writer-local `Float64QuarterAffineCsvPlan` for all-valid
Float64 typed CSV columns. It accepts only finite, nonnegative, exact
quarter-unit affine sequences and emits pandas-compatible fixed decimal text
from integer arithmetic. Any negative zero, negative value, NaN, infinity,
non-quarter value, non-affine sequence, overflow, or uncertified value keeps
the existing `write_pandas_float` fallback.

## Baseline

Command:

```bash
/data/projects/.scratch/cargo-target-orangepeak-uza0481-base/release-perf/high_ram_perf_baseline --profile uza0481-base --rows 100000 --iters 20 --warmup 3 --frame-cols 10 --key-cardinality 100000
```

CSV workload:

- mean: 86.04315275 ms
- p50: 86.642249 ms
- p95: 92.795965 ms
- p99: 94.571176 ms
- rows/sec: 1162207.5296398294
- payload bytes: 8611370
- rows_out: 100000
- checksum: 62503875000.0
- stable SHA: d93c23c308b1cbba82051d7dd022001e84e5313c0b485a440891a604ebfce367

## After

Command:

```bash
/data/projects/.scratch/cargo-target-orangepeak-uza0481-after/release-perf/high_ram_perf_baseline --profile uza0481-after --rows 100000 --iters 20 --warmup 3 --frame-cols 10 --key-cardinality 100000
```

CSV workload:

- mean: 47.4686875 ms
- p50: 46.718937 ms
- p95: 52.412132 ms
- p99: 52.78608 ms
- rows/sec: 2106651.884992607
- payload bytes: 8611370
- rows_out: 100000
- checksum: 62503875000.0
- stable SHA: d93c23c308b1cbba82051d7dd022001e84e5313c0b485a440891a604ebfce367

Delta:

- mean: 1.813x faster, 44.83% lower
- p50: 1.854x faster, 46.08% lower
- p95: 1.771x faster, 43.52% lower
- p99: 1.792x faster, 44.19% lower
- throughput: 1.813x higher

## Paired Hyperfine

Baseline-first full harness:

- baseline: 3.132 s +/- 0.067 s
- after: 2.440 s +/- 0.087 s
- ratio: after ran 1.28x +/- 0.05 faster

After-first reversed full harness:

- after: 2.362 s +/- 0.075 s
- baseline: 3.136 s +/- 0.106 s
- ratio: after ran 1.33x +/- 0.06 faster

Fresh paired artifacts:

- `tests/artifacts/perf/uza0481_pair_hyperfine_base_after_highram_keycard100000_100000x20.txt`
- `tests/artifacts/perf/uza0481_pair_hyperfine_base_after_highram_keycard100000_100000x20.json`
- `tests/artifacts/perf/uza0481_pair_hyperfine_after_base_highram_keycard100000_100000x20.txt`
- `tests/artifacts/perf/uza0481_pair_hyperfine_after_base_highram_keycard100000_100000x20.json`

## Isomorphism

- Row order, column order, header, delimiter, index exclusion, and quoting behavior are unchanged.
- Accepted values are never recomputed from floats during emission; the certified scaled integer is rendered directly.
- The plan rejects negative zero, negative finite values, NaN, infinity, non-quarter values, non-affine sequences, and checked arithmetic overflow.
- Fallback remains `write_pandas_float`, preserving pandas text for scientific notation, infinities, negative zero, and all uncertified floats.
- Stable CSV witness SHA, `rows_out`, `io_payload_bytes`, and checksum are unchanged.
- No RNG or tie-breaking surface is involved.

## Gates

- `cargo fmt -p fp-io -- --check`: pass
- `git diff --check -- crates/fp-io/src/lib.rs .skill-loop-progress.md tests/artifacts/perf/uza0481_pass1_baseline_profile_witness.md tests/artifacts/perf/uza0481_pass2_primitive_selection.md`: pass
- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0481-check rch exec -- cargo test -p fp-io quarter_affine --lib -- --nocapture`: pass on `vmi1153651`
- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0481-check rch exec -- cargo check -p fp-io --all-targets`: pass on `vmi1153651`
- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0481-clippy rch exec -- cargo clippy -p fp-io --all-targets -- -D warnings`: pass on `vmi1153651`
- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0481-after RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --bin high_ram_perf_baseline`: pass on `vmi1153651`
- `sha256sum -c tests/artifacts/perf/uza0481_after_csv_write_read_roundtrip_stable.sha256`: pass
- `timeout 240s ubs crates/fp-io/src/lib.rs`: exit 1 from existing file-wide inventory; no unsafe findings and no new quarter-affine hunk match in the panic/float-equality scan.

Fresh after artifacts:

- `tests/artifacts/perf/uza0481_after_high_ram_keycard100000_100000x20.json`
- `tests/artifacts/perf/uza0481_after_perf_highram_keycard100000_100000x20.json`
- `tests/artifacts/perf/uza0481_after_perf_report_no_children_highram_keycard100000_100000x20.txt`
- `tests/artifacts/perf/uza0481_after_perf_report_callgraph_highram_keycard100000_100000x20.txt`
- `tests/artifacts/perf/uza0481_ubs_fp_io.txt`

## After profile

After `perf record` captured 2556 samples with zero lost samples.
`ryu::pretty::format64` and `write_pandas_float` no longer appear in the
searched hot report. The residual shifted to:

- `fp_io::write_csv_string_with_options`: 23.62% children, 6.81% self
- `core::fmt::write`: 15.84% children, 3.27% self
- `<i64 as core::fmt::Display>::fmt`: 10.56% children, 2.61% self
- `<fp_frame::DataFrameGroupBy>::aggregate_named_func`: 11.89% children, 1.75% self

## Score

Impact 4 x Confidence 5 / Effort 3 = 6.67. Keep.
