# br-frankenpandas-uza04.56 cursor-free CSR rejection proof

## Target

- Bead: `br-frankenpandas-uza04.56`
- Baseline binary: `/data/projects/.scratch/cargo-target-orangepeak-uza0455-base/release-perf/examples/perf_profile`
- Candidate binary: `/data/projects/.scratch/cargo-target-orangepeak-uza0456-after/release-perf/examples/perf_profile`
- Scenario: `perf_profile outer_join 500000 20`
- Lever: replace the dense outer CSR builder's extra cursor allocation/copy with stable reverse-fill through the offsets array, then restore offsets.

## Behavior Proof

- Baseline golden: `tests/artifacts/perf/golden_base_uza0455_outer_join_20000.txt`
- Candidate golden: `tests/artifacts/perf/golden_after_uza0456_outer_join_20000.txt`
- Baseline sha256: `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`
- Candidate sha256: `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`
- `cmp -s` result: `golden_cmp=0`

Isomorphism obligations:

- Ordering: reverse-fill iterated input keys in reverse while decrementing bucket ends, which reconstructs the same ascending input-position order within every bucket as the baseline cursor fill.
- Tie-breaking: unchanged because bucket order and within-bucket position order were preserved.
- Floating point: unchanged; the lever only changed CSR position construction, not value casting or arithmetic.
- RNG/hash behavior: not involved.
- Null/NaN semantics: unchanged; matched, left-only, and right-only branch selection and sentinel handling were unchanged.

## Validation

- `cargo fmt -p fp-join`
- `rch exec -- cargo test -p fp-join merge_outer_dense_int64_duplicates_matches_generic_validated_route --lib`
- `rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- Golden SHA + `cmp -s`
- Paired hyperfine and longer confirmation hyperfine
- `cargo fmt -p fp-join -- --check` after removing the rejected hunk

## Benchmark Gate

Primary paired hyperfine (`outer_join 500000 20`, 12 runs):

- Baseline: `360.7 ms +/- 18.4`
- Candidate: `351.9 ms +/- 14.5`
- Ratio: candidate `1.03x +/- 0.07` faster

This was too noisy to keep, so a longer confirmation used `outer_join 500000 200`:

- Baseline: `2.451 s +/- 0.064`
- Candidate: `2.502 s +/- 0.054`
- Ratio: baseline `1.02x +/- 0.03` faster than candidate

Score: `0.0` (`Impact 0 x Confidence 4 / Effort 1`). The lever failed the `Score >= 2.0` keep gate.

## Decision

Rejected. The source hunk was removed, and no production code is retained.

Follow-up `br-frankenpandas-uza04.57` targets a different primitive: batched multi-lane value-tape construction or an equivalent columnar layout that avoids rescanning the dense outer plan separately for every promoted lane.
