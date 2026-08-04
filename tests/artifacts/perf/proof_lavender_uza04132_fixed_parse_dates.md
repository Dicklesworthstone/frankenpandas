# br-frankenpandas-uza04.132 fixed parse_dates proof

## Target

- Bead: `br-frankenpandas-uza04.132`
- Scenario: `csv_parse_dates_dt_year 100000x5`
- Profile route: `tests/artifacts/perf/lavender_next_routing_matrix.txt`
  listed this lane at `161.737 ms/iter`, ahead of the next residual.
- Lever: add one strict safe-Rust fast path for all-naive fixed-format CSV
  parse_dates values before the existing general `to_datetime` parser.

## One-lever boundary

Only `crates/fp-io/src/lib.rs` runtime behavior changed. The new helper accepts
these cells:

- `YYYY-MM-DD`
- `YYYY-MM-DD HH:MM:SS`
- missing/null values, emitted as `Timestamp::NAT`

Any other non-null cell returns `None` from the fast path and falls back to the
existing parser, including aware strings, mixed timezone strings, fractional
seconds, invalid dates, non-UTF8 scalars, and all-null columns.

## Isomorphism proof

- Ordering: one output scalar is pushed for each input scalar in input order.
- Tie-breaking: not applicable; no sort or grouping is introduced.
- Null/NaN: null cells remain datetime `NaT`; all-null columns still fall back
  because the helper requires at least one datetime string.
- Timezone/object fallback: `T`/`Z`, offsets, fractional seconds, and invalid
  calendar values are outside the fast-path grammar and use the previous parser.
- Floating point: not touched by this parser.
- RNG: no randomness.
- Safety: no unsafe code; arithmetic is checked for timestamp nanosecond
  construction.

Unit guard:

`rch exec -- cargo test -p fp-io csv_parse_dates_fixed_naive_fast_path_accepts_only_safe_domain -- --nocapture`

Result: `1 passed; 0 failed`.

Regression guard:

`rch exec -- cargo test -p fp-io csv_parse_dates_mixed_naive_and_aware_strings_normalizes_per_value -- --nocapture`

Result: `1 passed; 0 failed`.

## Golden output

Before:

```text
1442f9e26ae81e28ccfb59d5019f76af6c0306db9be04a084b339fb91fb6b90e  /tmp/lavender_uza04132_base_golden_csv_parse_dates_dt_year_1000.txt
cb6f4f743000dff6f84ca63c1c0f0e749c4be0c500dd9034402dcdb5321c7bdd  /tmp/lavender_uza04132_base_golden_csv_parse_dates_dt_year_100000.txt
```

After:

```text
1442f9e26ae81e28ccfb59d5019f76af6c0306db9be04a084b339fb91fb6b90e  /tmp/lavender_uza04132_after_golden_csv_parse_dates_dt_year_1000.txt
cb6f4f743000dff6f84ca63c1c0f0e749c4be0c500dd9034402dcdb5321c7bdd  /tmp/lavender_uza04132_after_golden_csv_parse_dates_dt_year_100000.txt
```

The before/after golden sha256 values are identical for both small and full
scenario sizes.

## Benchmark

Baseline from the pre-change release-perf binary:

- Artifact: `tests/artifacts/perf/lavender_uza04132_base_rch_baseline.txt`
- Mean: `1.272 s +/- 0.222 s`

Candidate from the post-change release-perf binary:

- Artifact: `tests/artifacts/perf/lavender_uza04132_after_candidate.txt`
- Mean: `530.1 ms +/- 23.4 ms`
- Internal: `109.510 ms/iter`

Paired current-load baseline:

- Artifact: `tests/artifacts/perf/lavender_uza04132_paired_base_current_load.txt`
- Mean: `1.790 s +/- 0.088 s`
- Internal: `375.959 ms/iter`

Paired current-load after:

- Artifact: `tests/artifacts/perf/lavender_uza04132_paired_after_current_load.txt`
- Mean: `512.1 ms +/- 11.4 ms`
- Internal: `114.329 ms/iter`

Paired speedup: `1.78996383648 / 0.5121403834 = 3.49x`.

## Acceptance

- Impact: 5
- Confidence: 5
- Effort: 2
- Score: `5 * 5 / 2 = 12.5`

Keep decision: accepted. The paired hyperfine win clears the Score>=2.0 gate
with unchanged golden sha256 values.

## Validation

- `rustfmt --edition 2024 --check crates/fp-io/src/lib.rs`: pass
- `cargo fmt --check`: fails on unrelated pre-existing workspace formatting
  drift outside this bead's touched source.
- `rch exec -- cargo check -p fp-io --all-targets`: pass
- `rch exec -- cargo clippy -p fp-io --all-targets -- -D warnings`: pass
- `rch exec -- cargo test -p fp-io csv_parse_dates_fixed_naive_fast_path_accepts_only_safe_domain -- --nocapture`: pass
- `ubs crates/fp-io/src/lib.rs`: nonzero from existing file-wide findings
  outside the new parser hunk; artifact
  `tests/artifacts/perf/lavender_uza04132_ubs_fp_io.txt`.

RCH notes: candidate build and crate-scoped validation used `rch exec`; some
commands failed open locally because workers were saturated. The proof keeps the
logs with the exact fallback output.

## Next profile target

Reprofile after this commit before selecting the next `[perf]` bead. Do not
retry the rejected `.dt.year` materialization family from `.131`; the parser
primitive moved the bottleneck.
