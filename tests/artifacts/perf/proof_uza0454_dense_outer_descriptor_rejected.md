# br-frankenpandas-uza04.54 dense outer descriptor rejection proof

## Target

- Bead: `br-frankenpandas-uza04.54`
- Commit baseline: `origin/main` at worktree start (`7ef3865c`)
- Profile target: `perf_profile outer_join 500000 20`
- Candidate lever: fused descriptor emission for dense single-key outer joins.

## Baseline

- Build command: `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0454-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- RCH status: crate-scoped command; all workers failed preflight and RCH failed open to local execution.
- Golden command: `/data/projects/.scratch/cargo-target-orangepeak-uza0454-base/release-perf/examples/perf_profile golden outer_join 20000`
- Golden sha256: `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`
- Baseline hyperfine: `370.9 ms +/- 10.0 ms` for `outer_join 500000 20` over 10 runs.

Profile evidence from the baseline `perf report`:

- `build_single_key_dense_i64_outer_merge_output`: `76.34%` children.
- `build_single_key_dense_i64_outer_all_matched_merge_output`: `17.82%` self.
- `build_single_key_dense_i64_outer_merge_output::{closure#0}`: `14.52%` self.
- `build_single_key_dense_i64_outer_merge_output::{closure#8}`: `10.88%` self.
- `build_single_key_dense_i64_outer_merge_output::{closure#6}`: `10.87%` self.

## Candidate

The tested lever fused descriptor emission during the dense bucket walk:

- Replaced the compact `(left_pos, right_start, run_len)` plan plus post-pass maps with direct `left_run_positions`, `run_lens`, `left_run_valid`, and `right_segments` vectors.
- Preserved the CSR bucket walk, bucket order, right segment tape, null sentinel, and lazy column constructors.
- Kept all semantics in safe Rust.

The candidate source hunk was removed after the benchmark gate failed.

## Isomorphism Proof

- Ordering: unchanged. The candidate emitted one descriptor at exactly the same bucket-walk points where the baseline pushed one plan tuple, so bucket order and left-within-bucket order were preserved.
- Tie-breaking: unchanged. Matched rows still iterate left CSR positions first and right bucket segments second; left-only and right-only rows retain the same sentinel branches.
- Floating point: unchanged. The candidate only changed descriptor construction; all numeric casts and column constructors were identical.
- RNG: not involved.
- Null/NaN semantics: unchanged. The `NONE_POS` sentinel and left/right validity flags were derived from the same matched, left-only, and right-only branches as the baseline.
- Golden output: baseline and candidate `perf_profile golden outer_join 20000` both produced sha256 `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`; `cmp -s` returned `golden_cmp=0`.

## Benchmark Gate

Paired hyperfine command:

`rch exec -- hyperfine --warmup 3 --runs 12 --export-json tests/artifacts/perf/hyperfine_pair_uza0454_outer_descriptor_base_after.json '/data/projects/.scratch/cargo-target-orangepeak-uza0454-base/release-perf/examples/perf_profile outer_join 500000 20' '/data/projects/.scratch/cargo-target-orangepeak-uza0454-after/release-perf/examples/perf_profile outer_join 500000 20'`

Result:

- Baseline: `386.3 ms +/- 74.2 ms`
- Candidate: `383.9 ms +/- 15.6 ms`
- Ratio: candidate `1.01x +/- 0.20` faster.

Score: `0.0` (`Impact 0 x Confidence 2 / Effort 1`). This fails the required `Score >= 2.0` keep gate.

## Validation

- `cargo fmt -p fp-join`
- `rch exec -- cargo test -p fp-join merge_outer_dense_int64_duplicates_matches_generic_validated_route --lib`
- `rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- `perf_profile golden outer_join 20000`
- `cmp -s` for baseline-vs-candidate golden output
- `rch exec -- hyperfine --warmup 3 --runs 12 ...`
- `cargo fmt -p fp-join -- --check` after removing the rejected hunk

## Next Route

Do not repeat the already rejected promoted i64-as-f64 lazy-lane family from `br-frankenpandas-uza04.52`, the nullable validity clearing family from `br-frankenpandas-uza04.50`, or this descriptor-remap fusion. The next profile-backed primitive should target the speculative dense outer all-matched staging cost or a deeper materialization boundary only after a fresh baseline/profile confirms it.
