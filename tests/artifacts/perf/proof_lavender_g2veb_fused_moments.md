# br-frankenpandas-g2veb proof - fused skew/kurt central moments

Agent: LavenderStone
Date: 2026-06-15

## Target

`Rolling::skew`, `Rolling::kurt`, `Expanding::skew`, and `Expanding::kurt`
computed the centered powers in separate iterator passes and then scanned the
same values again for the fully-constant pandas special case. The profile-backed
bead target was the O(n*w) / O(n^2) moment family.

## Rejected candidate

An online M2/M3/M4 expanding recurrence was tested first. It changed
display-level pandas golden output by a few ulps in `dataframe_expanding_skew`
and `dataframe_expanding_kurt`, so it was rejected. Artifacts:

- `lavender_g2veb_candidate_test_expanding_skew.txt`
- `lavender_g2veb_candidate_test_expanding_kurt.txt`
- `lavender_g2veb_candidate_test_series_expanding_skew_kurt.txt`

## Accepted lever

Fuse each skew/kurt central-moment loop:

- Keep the existing mean pass unchanged.
- Keep the existing `powi(2)`, `powi(3)`, and `powi(4)` terms unchanged.
- Accumulate `m2` and `m3` / `m4` in the same left-to-right value order as the
  old independent iterator sums.
- Fold the constant-window check into that same pass.

This removes one full scan for skew and kurt, plus the separate `all_same` scan,
without changing formulas, ordering, labels, null handling, RNG, or tie-breaking.

## Golden output proof

Baseline harness:

```text
n=12000 window=250
rolling_skew: 9.100 ms checksum 6f4ee13001d85e37
rolling_kurt: 7.548 ms checksum fcf2ade53dc4beb8
expanding_skew: 155.673 ms checksum fe7ed19352f5a8c5
expanding_kurt: 153.920 ms checksum 120ef936042f9b07
total_ms 326.242 combined_checksum ff53e5eaa40edb2d
```

Candidate harness:

```text
n=12000 window=250
rolling_skew: 6.507 ms checksum 6f4ee13001d85e37
rolling_kurt: 5.774 ms checksum fcf2ade53dc4beb8
expanding_skew: 113.377 ms checksum fe7ed19352f5a8c5
expanding_kurt: 115.935 ms checksum 120ef936042f9b07
total_ms 241.593 combined_checksum ff53e5eaa40edb2d
```

Normalized value ledger SHA-256:

```text
fe1a5e8a4d0fb2d9d1c890ed01428568ff60991b177fb6c959c8a91a41b0b8d5  tests/artifacts/perf/lavender_g2veb_base_value_hashes.txt
fe1a5e8a4d0fb2d9d1c890ed01428568ff60991b177fb6c959c8a91a41b0b8d5  tests/artifacts/perf/lavender_g2veb_candidate_value_hashes.txt
```

`lavender_g2veb_value_hashes.diff` is empty.

## Benchmark

Paired hyperfine, same machine, 8 runs:

```text
base:      334.1 ms +- 7.8 ms
candidate: 227.9 ms +- 8.0 ms
candidate ran 1.47 +- 0.06 times faster than base
```

Score: Impact 4 x Confidence 4 / Effort 1 = 16.0. Accepted.

## Validation

- `rustfmt --edition 2024 --check crates/fp-frame/src/lib.rs crates/fp-frame/examples/bench_window_moments.rs`: pass.
- `rch exec -- cargo test -p fp-frame expanding_skew -- --nocapture`: pass.
- `rch exec -- cargo test -p fp-frame expanding_kurt -- --nocapture`: pass.
- `rch exec -- cargo test -p fp-frame test_expanding_skew_kurt -- --nocapture`: pass.
- `rch exec -- cargo test -p fp-frame rolling_skew -- --nocapture`: pass via RCH local fallback when no workers were admissible.
- `rch exec -- cargo test -p fp-frame rolling_kurt -- --nocapture`: pass via RCH local fallback when no workers were admissible.
- `rch exec -- cargo check -p fp-frame --all-targets`: pass remotely on `vmi1227854`.
- `rch exec -- cargo clippy -p fp-frame --all-targets -- -D warnings`: pass remotely on `vmi1227854`.
- `ubs --only=rust crates/fp-frame/src/lib.rs crates/fp-frame/examples/bench_window_moments.rs`: timed out at 180s with `UBS_EXIT=124`; no findings were emitted before timeout.
