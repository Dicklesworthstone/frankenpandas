# br-frankenpandas-uza04.42 rejection proof

## Target

- Scenario: `perf_profile filter_bool 100000`.
- Baseline source: retained `.41` lazy strided Float64 filter rows binary.
- Profile evidence from `.41`: `Column::take_positions` remained visible at
  55.35% self / 55.58% children, with `DataFrame::loc_bool` at 25.21% self.
- Candidate primitive: compute one `PositionSelection` arithmetic-progression
  descriptor in `DataFrame::take_rows_by_positions_unchecked` and share it with
  every column, avoiding per-column rediscovery.

## Non-Repeat Boundary

This was not a fused-mask or eager-stride gather retry. It preserved the `.41`
lazy strided Float64 column representation and only tried to remove repeated
descriptor detection.

## Paired Benchmark

Compared binaries:

- Before: `/data/projects/.scratch/cargo-target-orangepeak-uza04-41/release-perf/examples/perf_profile`
- Candidate: `/data/projects/.scratch/cargo-target-orangepeak-uza04-42-before/release-perf/examples/perf_profile`

Results:

- `filter_bool 100000 20`: 50.5 ms +/- 1.8 ms before vs 51.2 ms +/- 3.2 ms candidate.
- `filter_bool 100000 1000`: 449.7 ms +/- 11.6 ms before vs 457.9 ms +/- 34.2 ms candidate.

Score: 0.0 because measured impact was negative. The keep gate was not met.

## Isomorphism Status

The candidate was behavior-preserving by construction, but it was not retained:

- Row order: unchanged, same `positions` slice.
- Duplicate/tie behavior: unchanged, descriptor was only used for strict
  arithmetic progressions.
- Floating point: unchanged, `.41` lazy Float64 materialization remained the
  data path.
- RNG: unchanged.
- Golden outputs: baseline `.42` goldens were captured and verified, but no
  after-golden was needed because the source hunk was rejected.

## Routing

Do not retry descriptor rediscovery, unchecked-position, fused-mask, or eager
stride variants. The next profile-backed child is `br-frankenpandas-uza04.43`:
lazy arithmetic-progression Int64 index labels for regular boolean filters.
