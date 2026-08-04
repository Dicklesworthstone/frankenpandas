# br-frankenpandas-0ezw7.1 proof: CSV parse_dates values-level datetime engine

Decision: KEEP.

## Profile-backed target

- Parent proof `br-frankenpandas-0ezw7` established that repeated `dt.year` on parsed
  `Datetime64(ns)` values is 2.03x-2.13x faster than reparsing Utf8 datetime strings.
- This follow-up targeted the CSV `parse_dates` producer boundary. Before this lever,
  `read_csv(parse_dates=[...])` built a temporary `Series` and `Index` around each
  datetime candidate column, then called the same `to_datetime` parser and copied
  `parsed.values()` back into the raw column vector.
- Local profiler attempts were blocked by host policy:
  `/proc/sys/kernel/perf_event_paranoid=4` denied `perf` and `samply`, and ptrace
  restrictions blocked `gdb` attach sampling. The denied profiler artifacts are kept
  under `tests/artifacts/perf/0ezw71_base_*`.

## One lever

Expose the existing `fp_frame::to_datetime_with_options` scalar loop as
`to_datetime_values_with_options(values, options)`, keep `Series` reconstruction in
`to_datetime_with_options`, and route `fp-io` CSV `parse_dates` / parse-date
combination paths directly through the values-level engine.

No datetime parsing rules were changed by this lever. CSV still uses
`infer_mixed_timezone=false` for per-value pandas-like parsing, and still falls back
to the original values if any non-missing input parses to missing.

## Golden output

Command shape:

```text
perf_profile golden csv_parse_dates_dt_year 5000
```

SHA256:

```text
a1aa0112ec4e38328e43f2fca1a3a0ff873ab26ee7b682ffd0f2eb92e5a91538  tests/artifacts/perf/0ezw71_base_golden_csv_parse_dates_dt_year_5000.txt
a1aa0112ec4e38328e43f2fca1a3a0ff873ab26ee7b682ffd0f2eb92e5a91538  tests/artifacts/perf/0ezw71_after_golden_csv_parse_dates_dt_year_5000.txt
cmp_status=0
```

## Benchmarks

Release-perf builds:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-0ezw71-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-0ezw71-after RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

RCH selected local fallback for the first baseline/after builds because no worker
was admissible at that moment; later crate-scoped checks and clippy ran remotely
on `vmi1227854`.

Short paired gate, `csv_parse_dates_dt_year 100000 5`:

- Baseline alone: 831.2 ms +/- 31.3 ms.
- Forward paired: baseline 836.5 ms +/- 22.3 ms, after 797.3 ms +/- 33.8 ms,
  after 1.05x +/- 0.05 faster.
- Reversed paired: after 779.8 ms +/- 23.1 ms, baseline 845.4 ms +/- 30.1 ms,
  after 1.08x +/- 0.05 faster.

Longer paired gate, `csv_parse_dates_dt_year 100000 20`:

- Forward paired: baseline 2.996 s +/- 0.098 s, after 2.846 s +/- 0.067 s,
  after 1.05x +/- 0.04 faster.
- Reversed paired: after 2.936 s +/- 0.154 s, baseline 3.028 s +/- 0.096 s,
  after 1.03x +/- 0.06 faster.

## Isomorphism proof

- Ordering preserved: yes. CSV row order, header order, parse-date combination
  insertion point, and index labels are unchanged. The lever only removes a
  temporary `Series`/`Index` wrapper inside parsing.
- Tie-breaking unchanged: N/A. No comparison, sorting, grouping, hashing, or
  duplicate-resolution logic changed.
- Floating-point unchanged: yes. Numeric CSV columns and datetime `dt.year`
  output are unchanged; golden output is byte-identical. The datetime parser
  used for numeric epochs is the same values loop that previously ran through
  `to_datetime_with_options`.
- RNG unchanged: N/A. The benchmark data generator is deterministic and uses no
  random seed.
- Null/NaT unchanged: yes. Existing `parse_failed` logic is preserved: if any
  non-missing original value becomes missing after parsing, CSV keeps the
  original unparsed values.
- Timezone boundary unchanged: yes. CSV still sets
  `infer_mixed_timezone=false`. Timezone-aware scalar values stay on the
  existing rendered offset-string path because the scalar model has no timezone
  metadata slot.

## Validation

- `cargo test -p fp-io csv_parse_dates --lib -- --nocapture`: passed.
- `cargo test -p fp-io csv_parse_date --lib -- --nocapture`: passed.
- `cargo test -p fp-frame --lib to_datetime -- --nocapture`: passed.
- `rch exec -- cargo check -p fp-frame --all-targets`: passed.
- `rch exec -- cargo check -p fp-io --all-targets`: passed on `vmi1227854`.
- `rch exec -- cargo check -p fp-conformance --example perf_profile`: passed on
  `vmi1227854`.
- `rch exec -- cargo clippy -p fp-frame --lib -- -D warnings`: passed on
  `vmi1227854`.
- `rch exec -- cargo clippy -p fp-io --all-targets -- -D warnings`: passed on
  `vmi1227854`.
- `rch exec -- cargo clippy -p fp-conformance --example perf_profile -- -D warnings`:
  passed on `vmi1227854`.
- `cargo fmt -p fp-frame -p fp-io -p fp-conformance -- --check`: failed on
  pre-existing broad rustfmt drift across examples and large test regions outside
  this lever. No source-file rustfmt sweep was included in this perf commit.
- `ubs crates/fp-frame/src/lib.rs crates/fp-io/src/lib.rs crates/fp-conformance/examples/perf_profile.rs`:
  attempted, but the scanner stalled for more than five minutes in `ast-grep`
  over the large `fp-frame` shadow copy and was terminated without findings.

## Additional rebased validation evidence

After this source/proof landed upstream, the same tree was rechecked with
separate RCH crate-scoped gates:

- `rch exec -- cargo test -j 1 -p fp-io csv_parse_dates --lib`: passed on
  `vmi1153651`.
- `rch exec -- cargo check -p fp-frame -p fp-io --all-targets`: passed on
  `vmi1227854`.
- `rch exec -- cargo clippy -p fp-frame -p fp-io --lib -- -D warnings`:
  passed on `vmi1227854`.

The rebased all-targets clippy attempt still fails only on pre-existing
`fp-frame` test-only `type_complexity` and `useless_vec` warnings around
`crates/fp-frame/src/lib.rs:82035+`. Rebased release-perf rebuild attempts were
not used for the keep decision: one RCH worker build went stale and was
canceled; a retry fell back local because no admissible worker slot was
available and was stopped.

## Score

Impact 2.0 x Confidence 4.0 / Effort 2.0 = 4.0. Keep.
