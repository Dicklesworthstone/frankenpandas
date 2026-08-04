# br-frankenpandas-uza04.131 csv parse_dates dt.year rejection

## Target

- Selection source: fresh current-main routing matrix from `cab5d7b5`.
- Top measured lane: `csv_parse_dates_dt_year 100000 5` at `161.737 ms/iter`.
- Baseline binary: `/data/projects/.scratch/cargo-target-lavender-next/release-perf/examples/perf_profile`.
- Candidate binary: `/data/projects/.scratch/cargo-target-lavender-uza04131-after/release-perf/examples/perf_profile`.

## Candidate

One lever was tested and then removed:

- Add `Column::as_datetime64_slice()` for all-valid typed Datetime64 columns.
- Use that slice in typed `.dt` component extraction to build Int64 output directly with `Column::from_i64_values`.
- Keep nullable, NaT, Utf8, and non-contiguous paths on the existing scalar fallback.

## Behavior Proof

The candidate preserved the golden outputs for `csv_parse_dates_dt_year`:

```text
1442f9e26ae81e28ccfb59d5019f76af6c0306db9be04a084b339fb91fb6b90e  n=1000
cb6f4f743000dff6f84ca63c1c0f0e749c4be0c500dd9034402dcdb5321c7bdd  n=100000
```

Checked artifacts:

- `tests/artifacts/perf/lavender_uza04131_base_golden.sha256`
- `tests/artifacts/perf/lavender_uza04131_after_golden.sha256`
- `tests/artifacts/perf/lavender_uza04131_base_golden_check.txt`
- `tests/artifacts/perf/lavender_uza04131_after_golden_check.txt`

Isomorphism:

- Row order and index labels unchanged.
- Output dtype remained `Int64`.
- Datetime parsing, timezone handling, parse failures, null/NaT behavior, and CSV row/column ordering were untouched.
- RNG and tie-breaking are not involved.

## Benchmark Gate

Baseline-only hyperfine:

```text
817.2 ms +/- 64.0 ms
```

After internal run:

```text
163.399 ms/iter
```

Paired forward:

```text
baseline:  774.0 ms +/- 13.2 ms
candidate: 803.4 ms +/- 22.3 ms
baseline ran 1.04 +/- 0.03 times faster
```

Paired reversed:

```text
candidate: 800.3 ms +/- 17.1 ms
baseline:  848.4 ms +/- 97.9 ms
candidate ran 1.06 +/- 0.12 times faster
```

Decision: reject. The result is order-sensitive, noisy, and below the Score >= 2.0 keep gate. The source hunk was removed.

## Next Route

The hot path is dominated upstream of typed `.dt.year()` materialization. Do not retry this accessor micro-lever. The next primitive should attack parse_dates itself, for example a direct fixed-format CSV datetime parser for the harness shape, while preserving pandas parse failure and mixed-timezone behavior.
