# br-frankenpandas-uza04.147 deferred parse_dates scalar proof

## Target

- Bead: `br-frankenpandas-uza04.147`
- Scenario: `csv_parse_dates_dt_year 100000x5`
- Baseline artifact: `tests/artifacts/perf/lavender_uza04147_base_hyperfine_csv_parse_dates_dt_year_100000x5.json`
- Current profile route: local release-perf matrix on `bb7c6523` had
  `csv_parse_dates_dt_year` as the top non-rejected residual after the
  `value_counts_nan50` residual closeout.

## One-lever boundary

The CSV options reader now defers scalar coercion for explicitly requested
single-column `parse_dates` columns. It still stores the raw field text for
pandas object-fallback parity, but it skips the intermediate `Scalar::Utf8`
allocation for the timestamp column while reading rows.

At `apply_csv_parse_dates`, the deferred column is parsed directly from raw
fields using the existing fixed-naive parser. If any field is outside the safe
fixed-naive grammar, the old scalar column is reconstructed from the raw fields
with the same `CsvReadOptions` and the existing general datetime parser runs.

## Isomorphism proof

- Ordering: every raw field is consumed in original CSV row order and the parsed
  column replaces the same column index.
- Tie-breaking: not applicable; no sort or grouping is introduced.
- Null/NaT: exact NA-token rules are preserved through `na_filter`,
  `keep_default_na`, and the custom NA set. Accepted NA fields become
  `Timestamp::NAT`, matching the previous fixed parser.
- Fallback: aware timestamps, mixed timezone/object cases, fractional seconds,
  invalid fixed dates, non-parseable strings, and all-null columns reconstruct
  the prior scalar column and use the existing parser.
- Floating point: numeric column parsing and promotion are unchanged.
- RNG: no randomness.
- Safety: safe Rust only; no unsafe code added.

Golden SHA:

```text
a1aa0112ec4e38328e43f2fca1a3a0ff873ab26ee7b682ffd0f2eb92e5a91538  tests/artifacts/perf/lavender_uza04147_base_golden_csv_parse_dates_dt_year_5000.txt
a1aa0112ec4e38328e43f2fca1a3a0ff873ab26ee7b682ffd0f2eb92e5a91538  tests/artifacts/perf/lavender_uza04147_candidate_golden_csv_parse_dates_dt_year_5000.txt
```

## Benchmark

Paired forward:

```text
baseline:  328.9 ms +/- 21.5 ms
candidate: 274.7 ms +/- 19.0 ms
speedup:   1.20x +/- 0.11
```

Paired reversed:

```text
candidate: 274.7 ms +/- 14.2 ms
baseline:  326.5 ms +/- 26.8 ms
speedup:   1.19x +/- 0.12
```

Artifacts:

- `tests/artifacts/perf/lavender_uza04147_pair_forward_csv_parse_dates_dt_year_100000x5.txt`
- `tests/artifacts/perf/lavender_uza04147_pair_forward_csv_parse_dates_dt_year_100000x5.json`
- `tests/artifacts/perf/lavender_uza04147_pair_reversed_csv_parse_dates_dt_year_100000x5.txt`
- `tests/artifacts/perf/lavender_uza04147_pair_reversed_csv_parse_dates_dt_year_100000x5.json`

## Acceptance

- Impact: 2.5
- Confidence: 4.0
- Effort: 2.0
- Score: `2.5 * 4.0 / 2.0 = 5.0`

Keep decision: accepted. The paired forward and reversed runs agree on a
roughly 19-20% latency reduction with unchanged golden output.

## Validation

- `RCH_REQUIRE_REMOTE=0 cargo test -j 1 -p fp-io csv_parse_dates_fixed_naive_fast_path_accepts_only_safe_domain --lib`: pass
- `RCH_REQUIRE_REMOTE=0 cargo test -j 1 -p fp-io csv_parse_dates_mixed_naive_and_aware_strings_normalizes_per_value --lib`: pass
- `RCH_REQUIRE_REMOTE=0 cargo check -j 1 -p fp-io --all-targets`: pass
- `RCH_REQUIRE_REMOTE=0 cargo clippy -j 1 -p fp-io --lib --no-deps -- -D warnings`: pass
- `rustfmt --edition 2024 --check crates/fp-io/src/lib.rs`: pass
- `git diff --check -- crates/fp-io/src/lib.rs tests/artifacts/perf/lavender_uza04147_* .skill-loop-progress.md .beads/issues.jsonl`: pass

Blocked/recorded:

- `RCH_REQUIRE_REMOTE=0 cargo clippy -j 1 -p fp-io --all-targets -- -D warnings`
  failed before reaching the touched crate on an unrelated `fp-frame`
  `clippy::manual_map` lint at `crates/fp-frame/src/lib.rs:12170`.
- `ubs crates/fp-io/src/lib.rs` returned the existing broad file-wide
  inventory: 27 critical, 5813 warnings, 719 info. The new hunk does not add
  unsafe code, panic macros, or unwrap/expect sites.
