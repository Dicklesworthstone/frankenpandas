# br-frankenpandas-9l6yv proof: ordered-unique outer Int64 key coalesce

## Target

- Bead: br-frankenpandas-9l6yv
- Hotspot: fp-join ordered-unique Int64 outer merge at 1M left rows / 1M right rows.
- Lever: when the shared outer-join key columns are all-valid Int64, coalesce the key output into a typed `Vec<i64>` and `Column::from_i64_values` instead of materializing `Scalar` values and running `Column::from_values`.

## Baseline

Built with:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavender-9l6yv-base rch exec -- cargo build --profile release-perf -p fp-join --bin join-bench
```

Internal probes:

```text
left  mean_ms=11.144 p50_ms=10.699 p95_ms=12.697 p99_ms=16.313 output_rows=1000000 checksum=150035204640.000
outer mean_ms=101.789 p50_ms=100.257 p95_ms=107.849 p99_ms=116.691 output_rows=1500000 checksum=200077920140.000
```

Baseline hyperfine:

```text
left  228.0 ms +/- 10.7 ms
outer 378.4 ms +/- 16.1 ms
```

## Candidate

Built with:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavender-9l6yv-candidate rch exec -- cargo build --profile release-perf -p fp-join --bin join-bench
```

Internal probes:

```text
left  mean_ms=11.392 p50_ms=11.363 p95_ms=13.202 p99_ms=13.421 output_rows=1000000 checksum=150035204640.000
outer mean_ms=48.453 p50_ms=46.947 p95_ms=55.846 p99_ms=57.446 output_rows=1500000 checksum=200077920140.000
```

Paired hyperfine, base then candidate:

```text
outer base      368.2 ms +/- 14.1 ms
outer candidate 320.8 ms +/-  7.3 ms
candidate ran 1.15 +/- 0.05 times faster
```

Paired hyperfine, candidate then base:

```text
outer candidate 322.1 ms +/- 10.4 ms
outer base      382.4 ms +/- 22.1 ms
candidate ran 1.19 +/- 0.08 times faster
```

Left non-regression spot check:

```text
left base      227.5 ms +/- 11.2 ms
left candidate 229.3 ms +/- 14.0 ms
base ran 1.01 +/- 0.08 times faster
```

## Golden output

Command:

```text
join-bench --ordered-unique --rows 5000 --right-rows 5000 --join-type outer --golden
```

SHA256:

```text
3da878cd8a0b40903116f5bc9d7e8687cf47af3d31d749f384d224582f9d9bce  baseline
3da878cd8a0b40903116f5bc9d7e8687cf47af3d31d749f384d224582f9d9bce  candidate
```

`cmp -s` between baseline and candidate golden dumps passed.

## Isomorphism

- Ordering: unchanged. The candidate consumes the existing `left_positions` / `right_positions` arrays in the same zip order as the scalar path.
- Tie-breaking: unchanged. For `(Some(left), Some(right))`, the candidate emits `left_keys[pos]`, matching the scalar path's `(Some(pos), _)` left-precedence rule.
- Null semantics: unchanged. The typed path is only used when both key columns expose all-valid Int64 buffers and every output slot has a valid left or right source. Any malformed `(None, None)` or out-of-range slot falls back to the existing scalar path.
- Floating point: unchanged. Only the shared Int64 key column materialization changes; payload columns still use the same nullable Float64-promoting outer reindex path.
- RNG: none.

## Validation

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavender-9l6yv-candidate rch exec -- cargo test -p fp-join merge_outer_ordered_unique_int64_subset_matches_generic_validated_route
PASS

CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavender-9l6yv-candidate rch exec -- cargo check -p fp-join --all-targets
PASS

CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavender-9l6yv-candidate rch exec -- cargo clippy -p fp-join --all-targets -- -D warnings
PASS

cargo fmt -p fp-join -- --check
PASS
```

`ubs crates/fp-join/src/lib.rs` completed but exited nonzero on pre-existing broad inventory in this file, including dtype equality false positives reported as secret comparisons and existing test unwrap/clone/direct-indexing warnings. No new finding was reported on the changed key-coalesce block.

## Score

Impact 3.0 x Confidence 0.9 / Effort 1.0 = 2.7. Keep.
