# br-frankenpandas-jbyuc.1.1.1.1.1.1.1.1.1.1 Proof

## Change

Profile-backed lever: ordered UTF8 join keys that expose matching lower-hex
contiguous sequence certificates now compute the equal-window overlap by
checked arithmetic before the byte-span lower-bound search. The helper compares
actual first-row prefix bytes as well as certificate shape before using numeric
interval intersection. Prefix/shape mismatch, non-lowerhex keys, nullable keys,
unsorted/duplicate keys, and arithmetic overflow all fall back to the existing
ordered UTF8 path.

Graveyard primitive: certificate-carrying monotone range intersection for a
sorted join key, closest to worst-case/sorted-iterator join seeking plus
vectorized/certificate execution.

## Baseline

- Head: `b2c0c0976e44f6bfc968f50efe7a4bec88f3b895`.
- Build: `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-jbyuc1111111111-base rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`; RCH failed open locally, but stayed crate-scoped in a scratch target dir.
- Golden baseline: `76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e`.
- Hyperfine baseline: `str_inner_join 1000000 1000000` mean `1.023 s +/- 0.056 s`.
- Baseline profile: `fp_join::utf8_span_lower_bound` `8.83%`, `build_single_key_inner_merge_output_with_selections` `6.71%`, `BTreeMap<String, Column>::insert` `2.45%`.

## Isomorphism

- Ordering preserved: yes. The arithmetic path returns the same increasing
  `(left_pos, right_pos)` contiguous overlap that byte-span lower bounds would
  produce for strict unique lower-hex sequences.
- Tie-breaking unchanged: yes. The path is gated by strict unique all-valid UTF8
  certificates. Duplicates or unsorted keys fall back to the existing planner.
- Floating-point/null behavior unchanged: yes. Only join position discovery
  changes; payload construction and `Column::take_contiguous_range` are unchanged.
  NaN/null/FP payload bits are selected from the same rows.
- RNG unchanged: yes. No RNG is involved.
- Prefix safety: actual prefix bytes must match. Same numeric lower-hex counters
  with different prefixes fall back.
- Overflow safety: interval math uses checked arithmetic and falls back on failure.
- Golden output: before/after files are byte-identical.

## Bench

- Paired A/B: baseline `1.416 s +/- 0.157 s`; after `657.3 ms +/- 18.7 ms`; after ran `2.15x +/- 0.25` faster.
- Reversed A/B: after `637.6 ms +/- 7.7 ms`; baseline `1.015 s +/- 0.032 s`; after ran `1.59x +/- 0.05` faster.
- Score: Impact 4 x Confidence 4 / Effort 2 = 8.0. Keep.

## Shifted Profile

After profile removed `utf8_span_lower_bound` from the visible hot list.
Top residuals shifted to output materialization and metadata:

- `build_single_key_inner_merge_output_with_selections`: `11.18%`.
- `BTreeMap<String, Column>::insert`: `5.19%`.
- `perf_profile::build_str_join_frame`: `4.67%`.
- `merge_single_key_inner_unsorted`: `3.84%`.
- `collect_join_key_columns`: `2.75%`.
- `Column::take_contiguous_range`: `2.63%`.

## Validation

- `cargo test -p fp-join ordered_unique_utf8_lower_hex --lib -- --nocapture`.
- `cargo test -p fp-join ordered_utf8_contiguous_no_overlap_output_fast_path_jbyuc111111111 --lib -- --nocapture`.
- `rustfmt --edition 2024 --check crates/fp-join/src/lib.rs`.
- `cargo check -p fp-join --all-targets`.
- `cargo clippy -p fp-join --all-targets -- -D warnings`.
- `ubs crates/fp-join/src/lib.rs`: completed. Reported broad pre-existing file-wide inventory/false positives outside the new helper; no unsafe blocks.

## Artifacts

- `tests/artifacts/perf/golden_base_jbyuc1111111111_str_inner_join_1000000.sha256`
- `tests/artifacts/perf/golden_after_jbyuc1111111111_str_inner_join_1000000.sha256`
- `tests/artifacts/perf/golden_compare_jbyuc1111111111.txt`
- `tests/artifacts/perf/hyperfine_base_jbyuc1111111111_str_inner_join_1000000x1000000.txt`
- `tests/artifacts/perf/hyperfine_pair_jbyuc1111111111_str_inner_join_1000000x1000000.txt`
- `tests/artifacts/perf/hyperfine_pair_reversed_jbyuc1111111111_str_inner_join_1000000x1000000.txt`
- `tests/artifacts/perf/perf_report_base_jbyuc1111111111_str_inner_join_1000000x1000000.txt`
- `tests/artifacts/perf/perf_report_after_jbyuc1111111111_str_inner_join_1000000x1000000.txt`
