# br-frankenpandas-uza04.68 keep proof: witness-backed dense domain guard

## Target

- Bead: `br-frankenpandas-uza04.68`
- Workload: `perf_profile outer_join 500000 {20,200}`
- Baseline commit: `4013e6fa` (`perf(fp-join): record rejected dense descriptor vectors`)
- Lever kept: consume exact cached `Int64DenseCycleWitness` metadata to derive dense key domains and skip the failed `outer_all_matched` pre-route when the domains provably differ.

## Profile-backed hotspot

Fresh baseline profile (`perf_report_base_uza0468_outer_join_500000x200.txt`) showed the dense int64 outer-join output route remained dominant:

- `build_single_key_dense_i64_outer_merge_output`: 28.01% self
- `build_single_key_dense_i64_outer_merge_output::{closure#3}`: 20.89% self
- `build_single_key_dense_i64_outer_all_matched_merge_output`: 12.41% self before returning `None` for the shifted-domain benchmark
- min/max scans remained visible in the real outer builder (`max<i64>` 4.18%, `min<i64>` 1.88%)
- tuple plan writes remained visible (`write<(usize, usize, usize)>` 3.84%, `push_mut` 2.29%)

## Change

Added `dense_cycle_domain(Int64DenseCycleWitness) -> Option<(i64, i64)>` in `fp-join` and used it in exactly two places:

1. `build_single_key_dense_i64_outer_all_matched_merge_output` now returns `None` immediately when both key columns have exact dense-cycle witnesses and their `(min, max)` domains differ.
2. `build_single_key_dense_i64_outer_merge_output` now computes the combined dense span from witness domains when both sides are certified, falling back to the existing full key scan otherwise.

The CSR and output emission logic is otherwise unchanged.

## Behavior proof

- Ordering preserved: yes. The bucket walk, CSR position order, left-major duplicate expansion, right segment order, and output column assembly are unchanged.
- Tie-breaking unchanged: yes. Witness domains only replace min/max discovery and an all-matched pre-route rejection; they do not reorder any row or bucket.
- Null placement unchanged: yes. `has_left_missing`, `has_right_missing`, sparse invalid ranges, and nullable lane constructors are still driven by the same bucket walk.
- DType promotion unchanged: yes. Promotion remains controlled by the same `has_left_missing` / `has_right_missing` flags after routing.
- Floating-point: unchanged. The only FP behavior remains the existing `i64 as f64` promotion path; no casts were changed.
- RNG/hash behavior: N/A. This route is deterministic and RNG-free.
- Fallback preserved: any absent, length-mismatched, overflowed, or non-dense-cycle witness uses the existing scan path.
- Golden output SHA-256 unchanged:
  - baseline: `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`
  - after: `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`
- `sha256sum -c` passed for saved base and after golden files.
- Byte compare: `golden_cmp=0`.

## Benchmark evidence

Baseline standalone:

- `outer_join 500000x20`: 0.294s total, 14.693 ms/iter
- `outer_join 500000x200`: 1.810s total, 9.052 ms/iter
- baseline hyperfine `500000x20`: 246.7 ms +/- 18.3
- baseline hyperfine `500000x200`: 1.660 s +/- 0.181

Candidate standalone:

- `outer_join 500000x20`: 0.225s total, 11.257 ms/iter
- `outer_join 500000x200`: 1.151s total, 5.755 ms/iter

Paired hyperfine:

- `500000x20`: baseline 223.0 ms +/- 15.4, after 192.8 ms +/- 20.3; after was 1.16x +/- 0.15 faster.
- `500000x200`: baseline 1.909 s +/- 0.085, after 1.244 s +/- 0.053; after was 1.53x +/- 0.09 faster.

Score: Impact 5 x Confidence 5 / Effort 3 = 8.33. Keep.

## Validation

- `cargo fmt -p fp-join -- --check`
- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0468-test-join rch exec -- cargo test -p fp-join merge_outer_dense_int64_duplicates_matches_generic_validated_route --lib`
- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0468-check rch exec -- cargo check -p fp-join --all-targets`
- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0468-clippy rch exec -- cargo clippy -p fp-join --all-targets -- -D warnings`
- `ubs crates/fp-join/src/lib.rs .skill-loop-progress.md .beads/issues.jsonl tests/artifacts/perf/proof_uza0468_witness_domain_keep_orangepeak.md`

`rch` selected remote `vmi1227854` for the baseline build, then failed open locally for the focused test/check/clippy/after build because no worker slot was admissible. Benchmarks were paired locally against the saved baseline and after binaries.

UBS exited nonzero on broad pre-existing `fp-join` inventory/false positives (test unwraps, dtype comparisons flagged as secret comparisons, existing clone/indexing inventories). No actionable finding was introduced by the witness-domain hunk; `cargo check`, clippy, fmt, and the focused parity test passed.

## After profile

After profile (`perf_report_after_uza0468_outer_join_500000x200.txt`) removed the failed all-matched pre-route from the visible hot list. The top residual shifted to actual outer plan/CSR work:

- `build_single_key_dense_i64_outer_merge_output`: 18.50% self
- `build_single_key_dense_i64_outer_merge_output::{closure#5}`: 18.14% self
- tuple plan write/push remains visible (`write<(usize, usize, usize)>` 7.59%, `push_mut` 3.29%)

Next target should attack the remaining plan/CSR emission with a different primitive than the rejected `.67` direct parallel descriptor vectors.
