# br-frankenpandas-uza04.147 proof - deferred parse_dates scalar inference

## Target

- Bead: `br-frankenpandas-uza04.147`
- Hotspot: `csv_parse_dates_dt_year 100000x5`
- Profile-backed route: current local release-perf matrix on `bb7c6523` showed `csv_parse_dates_dt_year 100000x5` at `78.438 ms/iter`, top non-rejected residual after the `0mtyz` hash-table rejection.
- Prior wins not repeated: values-level datetime engine (`0ezw7.1`) and strict fixed CSV datetime parser (`uza04.132`).

## Primitive

- Graveyard/data-plane primitive: parser projection pushdown / deferred semantic classification.
- Alien artifact: a replayable parse witness boundary. During row ingestion, columns named in `parse_dates` keep their raw field strings and receive placeholder scalar cells. At the parse_dates materialization boundary, the code either parses fixed naive datetimes directly from raw fields or replays the previous scalar inference from raw fields and calls the existing general parse_dates path.
- One lever only: move scalar inference for parse_dates columns to the semantic boundary. No datetime grammar expansion, no `dt.year` change, no value-column inference change, no row scheduler change.

## Baseline

- Baseline binary: `/data/projects/.scratch/cargo-target-lavender-current/release-perf/examples/perf_profile`
- Baseline golden 5000 SHA256:
  `a1aa0112ec4e38328e43f2fca1a3a0ff873ab26ee7b682ffd0f2eb92e5a91538`
- Baseline unpaired local hyperfine:
  `392.8 ms +/- 22.0 ms` for `csv_parse_dates_dt_year 100000 5`
  in `tests/artifacts/perf/lavender_uza04147_base_hyperfine_csv_parse_dates_dt_year_100000x5.json`.

## Behavior Proof

- Ordering preserved: yes. CSV records are still appended in reader order. Header order and `usecols` filtering are unchanged; the deferred mask is zipped through the same `headers`, `columns`, and `raw_columns` filter.
- Tie-breaking unchanged: N/A for this workload. No sorting or grouping logic changed.
- Floating point unchanged: yes. Numeric value columns still use the same `parse_scalar_with_options` path and the benchmark output is integer year extraction.
- RNG unchanged: N/A. The workload has deterministic generated CSV input and no RNG.
- Null/NaT preserved: yes. Default pandas NA and custom `na_values` in raw parse-date fields map to `Datetime64(NaT)`. `keep_default_na=false` and failed parse fallback replay the old scalar parser instead of leaking placeholders.
- Timezone/object fallback preserved: yes. The direct raw fast path accepts only the existing fixed naive parser domain. Mixed naive/aware, fractional seconds, invalid dates, and object fallback use the old scalar replay plus `parse_csv_datetime_values` path.
- SQL path preserved: yes. SQL parse_dates still calls `apply_parse_dates_to_scalar_columns`, which is the old scalar-column behavior under a new name.
- Parse-date combinations preserved: yes. Deferral is disabled when parse-date combinations or named combinations are present.

Golden verification:

- Candidate 5000 SHA256:
  `a1aa0112ec4e38328e43f2fca1a3a0ff873ab26ee7b682ffd0f2eb92e5a91538`
- 5000 diff: `tests/artifacts/perf/lavender_uza04147_candidate_golden_5000_diff.txt`, `0` lines.
- Baseline/candidate 100000 SHA256:
  `cb6f4f743000dff6f84ca63c1c0f0e749c4be0c500dd9034402dcdb5321c7bdd`
- 100000 diff: `tests/artifacts/perf/lavender_uza04147_candidate_golden_100000_diff.txt`, `0` lines.

Focused tests:

- `cargo test -j 1 -p fp-io csv_parse_dates --lib -- --nocapture`: passed, 6 tests.
- `cargo test -j 1 -p fp-io read_csv_with_options_object_fallback_preserves_text --lib -- --nocapture`: passed.
- New tests cover raw custom-NA `NaT`, fallback replay with invalid dates and `keep_default_na=false`, and `usecols`/headerless deferred-mask alignment.

Validation:

- `rustfmt --edition 2024 crates/fp-io/src/lib.rs --check`: passed.
- `cargo check -j 1 -p fp-io --lib`: passed.
- `cargo clippy -j 1 -p fp-io --lib -- -D warnings`: blocked by unrelated path-dependency lint in `crates/fp-frame/src/lib.rs:12170` (`clippy::manual_map`).
- `cargo clippy -j 1 -p fp-io --lib --no-deps -- -D warnings`: passed.
- `perf stat` profiling: blocked by host policy, `perf_event_paranoid=4`.

## Rebench

Forward paired hyperfine:

- Base: `384.8 ms +/- 96.9 ms`
- Candidate: `269.1 ms +/- 10.5 ms`
- Ratio: `1.43x +/- 0.36` faster
- Artifact: `tests/artifacts/perf/lavender_uza04147_pair_hyperfine_csv_parse_dates_dt_year_100000x5.txt`

Reversed paired hyperfine:

- Candidate: `266.1 ms +/- 8.8 ms`
- Base: `345.0 ms +/- 14.7 ms`
- Ratio: `1.30x +/- 0.07` faster
- Artifact: `tests/artifacts/perf/lavender_uza04147_pair_reversed_hyperfine_csv_parse_dates_dt_year_100000x5.txt`

## Score

- Impact: 4 (30-43% full-chain local speedup for the profiled residual)
- Confidence: 5 (forward and reversed paired wins, byte-identical goldens at 5000 and 100000, focused tests, check, no-deps clippy)
- Effort: 2 (narrow `fp-io` parser-boundary lever)
- Score: `4 * 5 / 2 = 10.0`
- Verdict: keep.
