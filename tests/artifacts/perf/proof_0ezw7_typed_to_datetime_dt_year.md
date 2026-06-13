# br-frankenpandas-0ezw7 proof: typed naive to_datetime backing for dt.year

## Decision

KEEP. The single lever converts naive and numeric `to_datetime` outputs from
rendered `Utf8` timestamp scalars to typed `Datetime64(ns)` scalars, while
preserving the existing UTC/timezone-aware string paths. This removes repeated
string parsing from `parsed.dt().year()` and uses the already-routed typed
datetime accessor path.

Score: Impact 5 x Confidence 5 / Effort 2 = 12.5.

## Profile-backed target

`br-frankenpandas-0ezw7` records that datetime values stored as Utf8 force
`dt.year/month/day/hour/...` to re-parse every call. The benchmark added here
parses a deterministic datetime string Series once with `to_datetime`, then
times repeated `parsed.dt().year()` calls.

Scenario: `perf_profile dt_year 200000 100`.

## Golden output proof

Command family:

```bash
BASE=/data/projects/.scratch/cargo-target-orangepeak-0ezw7-base/release-perf/examples/perf_profile
AFTER=/data/projects/.scratch/cargo-target-orangepeak-0ezw7-after/release-perf/examples/perf_profile
$BASE golden dt_year 5000 > tests/artifacts/perf/0ezw7_base_golden_dt_year_5000.txt
$AFTER golden dt_year 5000 > tests/artifacts/perf/0ezw7_after_golden_dt_year_5000.txt
sha256sum tests/artifacts/perf/0ezw7_base_golden_dt_year_5000.txt tests/artifacts/perf/0ezw7_after_golden_dt_year_5000.txt
cmp -s tests/artifacts/perf/0ezw7_base_golden_dt_year_5000.txt tests/artifacts/perf/0ezw7_after_golden_dt_year_5000.txt
```

SHA256:

```text
a1aa0112ec4e38328e43f2fca1a3a0ff873ab26ee7b682ffd0f2eb92e5a91538  tests/artifacts/perf/0ezw7_base_golden_dt_year_5000.txt
a1aa0112ec4e38328e43f2fca1a3a0ff873ab26ee7b682ffd0f2eb92e5a91538  tests/artifacts/perf/0ezw7_after_golden_dt_year_5000.txt
cmp_status=0
```

## Benchmark proof

Baseline binary was built from
`/data/projects/.scratch/frankenpandas-orangepeak-0ezw7-base` with the harness
addition only:

```bash
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-0ezw7-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

After binary was built from
`/data/projects/.scratch/frankenpandas-orangepeak-0ezw7-after`:

```bash
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-0ezw7-after RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Forward paired hyperfine, `dt_year 200000 100`:

```text
baseline mean 1.51517127754 s, median 1.51110900754 s
after    mean 0.71101043194 s, median 0.70886740754 s
after ran 2.13 +/- 0.11 times faster than baseline
```

Reversed paired hyperfine, `dt_year 200000 100`:

```text
after    mean 0.72855290502 s, median 0.72912192272 s
baseline mean 1.47553414452 s, median 1.46878650722 s
after ran 2.03 +/- 0.09 times faster than baseline
```

Artifacts:

- `tests/artifacts/perf/0ezw7_pair_forward_hyperfine_dt_year_200000x100.json`
- `tests/artifacts/perf/0ezw7_pair_forward_hyperfine_dt_year_200000x100.txt`
- `tests/artifacts/perf/0ezw7_pair_reversed_hyperfine_dt_year_200000x100.json`
- `tests/artifacts/perf/0ezw7_pair_reversed_hyperfine_dt_year_200000x100.txt`

## Isomorphism proof

- Ordering: input index order is preserved; only the internal scalar dtype for
  naive/numeric datetimes changes from normalized string to `Datetime64(ns)`.
- Tie-breaking: not applicable; `to_datetime` is elementwise and deterministic.
- Floating point: numeric epoch floats still truncate through the same `as i64`
  path before timestamp conversion. No new floating reductions or FMA-sensitive
  operations are introduced.
- RNG: not applicable.
- Null/NaT: missing, NaN, infinite, overflow, and parse-failure paths continue
  to produce `Scalar::Null(NullKind::NaT)` or the previous coercion fallback.
- Timezones and `utc=true`: timezone-aware strings and explicit UTC conversion
  retain the existing rendered string path because the current scalar model has
  no timezone metadata slot. The typed path is intentionally limited to
  timezone-naive instants.
- Golden output: `dt_year` golden output is byte-identical by SHA256 and `cmp`.

## Validation

Passed:

```bash
rch exec -- cargo test -p fp-frame --lib to_datetime -- --nocapture
rch exec -- cargo check -p fp-frame --all-targets
rch exec -- cargo clippy -p fp-frame --all-targets -- -D warnings
rch exec -- cargo check -p fp-conformance --example perf_profile
git diff --check
```

Notes:

- `cargo clippy -p fp-frame --all-targets -- -D warnings` was invoked through
  `rch`, but `rch` fell open to local execution because no worker passed
  preflight at that moment. It completed successfully.
- `cargo fmt -p fp-frame -p fp-conformance -- --check` still reports
  pre-existing formatting drift in unrelated examples and older sections of
  `fp-frame/src/lib.rs`. The new helper/test hunks were manually aligned to
  rustfmt's emitted shape where they appeared in the diff.
- `timeout 180s ubs crates/fp-frame/src/lib.rs crates/fp-conformance/examples/perf_profile.rs`
  timed out before producing findings.
