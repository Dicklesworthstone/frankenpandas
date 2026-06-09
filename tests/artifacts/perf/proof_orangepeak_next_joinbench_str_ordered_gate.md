# br-frankenpandas-uza04.45 Proof

## Change

Added a setup-free `join-bench --str-ordered` scenario for ordered lower-hex
UTF8 inner joins. The benchmark constructs sorted, all-valid UTF8 key columns
once, then times only `merge_dataframes*` calls and reports a checksum over the
merged output so the result stays behavior-observable.

This is a measurement gate only. It does not alter production join semantics.

## Baseline And Golden

- Build: `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-next-join-bench RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-join --profile release-perf --bin join-bench`; RCH failed open locally, crate-scoped.
- Gate command: `join-bench --str-ordered --rows 1000000 --right-rows 1000000 --join-type inner --warmup 3 --iters 20`.
- Hyperfine gate baseline: `124.0 ms +/- 3.1 ms` for the full command.
- Gate golden SHA: `2ac49173153820d4b3878817c44be31979faa18b2ae034167f7977adee83b02e`.
- Existing `perf_profile str_inner_join` golden SHA for the same row scale: `76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e`.

## Profile Finding

`perf_report_before_orangepeak_next_joinbench_str_ordered_1m.txt` showed the
join-only timed path is no longer the certificate-seek bottleneck for this
ordered UTF8 case. The visible self-costs are dominated by checksum and teardown:

- `Scalar::to_f64`: `42.11%`.
- `join_bench::main`: `29.82%`.
- `drop_in_place::<fp_columnar::Column>`: `7.02%`.

This means the next production lever should not retry lower-hex seek/certificate
work. The profile-backed route is the already-open child:
`br-frankenpandas-jbyuc.1.1.1.1.1.1.1.1.1.1.1`, targeting compact ordered UTF8
join output materialization and metadata insertion.

## Isomorphism

- Ordering/tie behavior: unchanged; benchmark-only frame construction uses
  monotonically increasing unique keys and calls the existing merge APIs.
- Null/NaN/FP behavior: unchanged; production code is untouched.
- RNG: unchanged; deterministic arithmetic values only.
- Golden output: gate output hash recorded for future regression checks.

## Validation

- `cargo fmt -p fp-join -- --check`.
- `cargo check -p fp-join --all-targets`.
- `cargo clippy -p fp-join --all-targets -- -D warnings`.
- `ubs crates/fp-join/src/bin/join-bench.rs`.

## Artifacts

- `tests/artifacts/perf/golden_before_orangepeak_next_joinbench_str_ordered_1m.sha256`
- `tests/artifacts/perf/hyperfine_before_orangepeak_next_joinbench_str_ordered_1m.txt`
- `tests/artifacts/perf/perf_report_before_orangepeak_next_joinbench_str_ordered_1m.txt`
- `tests/artifacts/perf/ubs_orangepeak_next_joinbench_str_ordered_gate.txt`
