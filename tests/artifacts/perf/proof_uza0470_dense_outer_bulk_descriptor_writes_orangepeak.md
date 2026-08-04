# br-frankenpandas-uza04.70 keep proof: dense outer bulk descriptor writes

## Target

- Bead: `br-frankenpandas-uza04.70`
- Workload: `perf_profile outer_join 500000 {20,200}`
- Baseline head: `913044d1`
- Lever kept: exact-capacity dense outer run-tape descriptors now use bulk `repeat_n` extension for repeated run lengths, left-run validity, and right segments, and the dense-cycle position appender no longer re-reserves per bucket. The fallback CSR/tuple-plan route is unchanged.

## Profile-backed hotspot

Fresh baseline profile (`perf_report_base_uza0470_outer_join_500000x200_nochildren.txt`) showed the post-`.69` residual:

- `build_dense_cycle_outer_run_tape`: `59.01%` self.
- Visible descriptor-write costs: `push_mut<usize>` / `push<usize>` at `5.11%`, `push<usize>` at `4.09%`, `push_mut<(usize, usize)>` at `2.62%`, `push_mut<bool>` at `2.01%`, plus iterator `spec_next`.
- Target shape: certified dense-cycle partial-overlap outer join, preserving exact bucket order and generic fallback behavior.

## Change

The dense-cycle tape builder still emits the same descriptor representation:

- `run_lens`
- `left_run_valid`
- `left_run_positions`
- `right_positions_csr`
- `right_segments`
- `key_runs`
- sparse invalid ranges

Only the write pattern changed. Repeated descriptors are appended in batches with exact capacity instead of one `push` per descriptor lane, and `append_dense_cycle_positions` trusts the caller's exact capacity rather than calling `reserve` for every key bucket.

## Behavior proof

- Ordering: unchanged. The bucket walk is still ascending key order, and left positions are still emitted as `offset + k * period`.
- Tie-breaking: unchanged. Matched buckets still keep one run per left row, each referencing the same right bucket-order segment.
- Null placement: unchanged. Left-only and right-only invalid ranges are appended at the same output offsets as before.
- DType promotion: unchanged. `has_left_missing` and `has_right_missing` drive the same lane constructors.
- Floating point: unchanged. The only f64 behavior remains the existing `i64 as f64` nullable promotion path.
- RNG/hash behavior: unchanged. This path is deterministic and hash-free.
- Golden `outer_join 20000` SHA stayed `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`.
- Byte compare: `golden_cmp=0`.

## Benchmark evidence

Direct harness timing:

- `outer_join 500000x20`: `7.468 ms/iter` -> `7.259 ms/iter`.
- `outer_join 500000x200`: `3.308 ms/iter` -> `2.756 ms/iter`.

Paired hyperfine:

- `500000x20`: baseline `143.6 ms +/- 7.6`, after `125.5 ms +/- 8.5`; after `1.14x +/- 0.10` faster.
- `500000x200`: baseline `768.8 ms +/- 75.2`, after `611.9 ms +/- 97.5`; after `1.26x +/- 0.23` faster.

Score: Impact 4 x Confidence 3 / Effort 2 = 6.0. Keep.

## Validation

- `cargo fmt -p fp-join -- --check`: passed.
- `rch exec -- cargo test -p fp-join merge_outer_dense_int64_duplicates_matches_generic_validated_route --lib`: passed.
- `rch exec -- cargo check -p fp-join --all-targets`: passed.
- `rch exec -- cargo clippy -p fp-join --all-targets --no-deps -- -D warnings`: passed.
- Full `cargo clippy -p fp-join --all-targets -- -D warnings` was attempted and failed in upstream `fp-frame` at `crates/fp-frame/src/lib.rs:45570` (`needless_range_loop`) before linting this `fp-join` change.
- `ubs crates/fp-join/src/lib.rs .skill-loop-progress.md .beads/issues.jsonl tests/artifacts/perf/proof_uza0470_dense_outer_bulk_descriptor_writes_orangepeak.md`: exited nonzero on pre-existing broad `fp-join` inventory (`3` critical, `3840` warning, `247` info); no finding targets the changed `repeat_n`/reserve hunk.

## After profile

Final after profile (`perf_report_after_uza0470_outer_join_500000x200_nochildren.txt`) still leaves `build_dense_cycle_outer_run_tape` as the top residual at `48.26%` self. The next route should stop tuning repeated descriptor lanes and attack the remaining parametric dense-cycle position tape / descriptor materialization boundary directly, likely by avoiding the right-position CSR tape or by adding a descriptor-free columnar lazy primitive for certified dense-cycle outer joins.
