# br-frankenpandas-uza04.141 proof - Datetime64 dt.year typed-slice Int64 fast path

## Decision

Accepted.

`br-frankenpandas-uza04.139` was closed externally as a duplicate while this
measurement was in progress. The live bead for the retained source change is
`br-frankenpandas-uza04.141`.

## Target

Current route artifact `tests/artifacts/perf/lavenderstone_agent_route_current_matrix.txt`
showed `csv_parse_dates_dt_year 100000x5` at `63.675 ms/iter`, second only to
the rejected dense `df_dot` lane. The path ends in typed `Datetime64.dt.year`.

## One Lever

Expose a typed `datetime64[ns]` input slice from `fp-columnar` and specialize
`fp-frame` `DatetimeAccessor::year()` for all-valid Datetime64 columns:

- `Column::as_datetime64_slice()` borrows the cached typed nanos buffer.
- `DatetimeAccessor::year()` uses direct civil-year arithmetic and emits
  `Column::from_i64_values`.
- Any `NaT` falls back to the pre-existing generic extractor so missing
  `NullKind::NaN` behavior is preserved.

No ordering, tie-breaking, floating-point, or RNG behavior is involved.

## Golden Proof

Baseline and candidate golden outputs were captured for:

- `csv_parse_dates_dt_year 2000`
- `csv_parse_dates_dt_year 5000`
- `dt_year 5000`

All output diffs are empty:

- `tests/artifacts/perf/uza04139_golden_diff_csv_2000.txt`
- `tests/artifacts/perf/uza04139_golden_diff_csv_5000.txt`
- `tests/artifacts/perf/uza04139_golden_diff_dt_year_5000.txt`
- `tests/artifacts/perf/uza04139_golden_sha256_only_diff.txt`

The artifact prefix remains `uza04139` because `.139` was the active bead when
the baseline/candidate bundles were generated; `.141` is the live carry-forward
bead for the distinct accepted lever.

## Benchmark Proof

RCH was used for build commands; workers repeatedly failed open to local or
were unavailable for some commands, so timing proof is same-machine paired
hyperfine from isolated target directories.

Full route, `csv_parse_dates_dt_year 100000 5`:

- Forward: base `405.1 ms +/- 23.2`, candidate `388.2 ms +/- 19.9`,
  candidate `1.04x +/- 0.08` faster.
- Reversed: candidate `420.3 ms +/- 31.4`, base `460.9 ms +/- 24.3`,
  candidate `1.10x +/- 0.10` faster.

Isolated typed accessor, `dt_year 100000 50`:

- Forward: base `282.4 ms +/- 10.7`, candidate `226.3 ms +/- 11.7`,
  candidate `1.25x +/- 0.08` faster.
- Reversed: candidate `242.4 ms +/- 17.8`, base `277.9 ms +/- 9.9`,
  candidate `1.15x +/- 0.09` faster.

Score: Impact 3 * Confidence 4 / Effort 2 = `6.0`; accepted.

## Validation

- `rustfmt --check --edition 2024 crates/fp-frame/src/lib.rs crates/fp-columnar/src/lib.rs`
- `rch exec -- cargo check -p fp-columnar -p fp-frame --lib`
- `rch exec -- cargo clippy -p fp-columnar -p fp-frame --lib -- -D warnings`
- `rch exec -- cargo test -p fp-frame dt_year --lib`

`cargo check -p fp-frame -p fp-columnar --all-targets` was attempted and
blocked by unrelated untracked `crates/fp-columnar/examples/bench_cmp_f64.rs`
with no `main` function. `cargo fmt -p fp-frame -p fp-columnar --check` was
also blocked by unrelated package example formatting. Direct rustfmt on touched
source files passed.

`ubs crates/fp-frame/src/lib.rs crates/fp-columnar/src/lib.rs` did not finish;
the first run was terminated after more than five minutes, and the bounded
rerun timed out after 180 seconds. The captured artifact is
`tests/artifacts/perf/uza04139_ubs_touched_sources.txt` and contains only UBS
startup output before timeout.
