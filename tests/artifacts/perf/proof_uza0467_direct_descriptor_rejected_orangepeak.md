# br-frankenpandas-uza04.67 rejection proof: direct descriptor vectors

## Target

- Bead: `br-frankenpandas-uza04.67`
- Workload: `perf_profile outer_join 500000 {20,200}`
- Baseline commit: `5fdb6dbe` (`perf(fp-join): cache dense-cycle int64 join witnesses`)
- Lever attempted: replace the dense-cycle outer-join temporary tuple `plan` with parallel descriptor vectors emitted directly during the bucket walk.

## Profile-backed hotspot

Baseline profile (`perf_report_base_uza0467_outer_join_500000x200.txt`) kept the target inside
`build_single_key_dense_i64_outer_merge_output`:

- `build_single_key_dense_i64_outer_merge_output`: 24.38% self
- `build_single_key_dense_i64_outer_merge_output::{closure#3}`: 15.87% self
- tuple plan push/write machinery was visible in the profile (`Vec<(usize, usize, usize)>` writes and pushes).

## Behavior proof

- Ordering preserved: yes. The rejected candidate kept the same bucket walk order: increasing dense bucket, then left CSR position order, then right segment order.
- Tie-breaking unchanged: yes. Duplicate keys retained the same left-row-major expansion and the same right run segment.
- Floating-point: N/A. The benchmarked key path is dense int64 outer join; no FP arithmetic changed.
- RNG seeds: N/A. No randomized logic.
- Golden output SHA-256 unchanged:
  - baseline: `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`
  - after: `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`
- Byte compare: `cmp -s golden_base_uza0467_outer_join_20000.txt golden_after_uza0467_outer_join_20000.txt` passed.

## Benchmark evidence

Baseline standalone:

- `outer_join 500000 20`: 0.204s total, 10.220 ms/iter
- `outer_join 500000 200`: 1.129s total, 5.644 ms/iter
- baseline hyperfine `500000x20`: 207.8 ms +/- 27.4
- baseline hyperfine `500000x200`: 1.199 s +/- 0.054

Candidate standalone:

- `outer_join 500000 20`: 0.211s total, 10.553 ms/iter
- `outer_join 500000 200`: 1.311s total, 6.554 ms/iter

Paired hyperfine:

- `500000x20`: baseline 201.4 ms +/- 8.9, candidate 235.7 ms +/- 20.0; baseline was 1.17x +/- 0.11 faster.
- `500000x200`: baseline 1.297 s +/- 0.082, candidate 1.380 s +/- 0.056; baseline was 1.06x +/- 0.08 faster.

## Verdict

Rejected. Impact is negative on both paired workloads, so the optimization score is below the `>= 2.0` keep threshold. The code lever was removed; only this evidence artifact and bead/progress metadata are retained.

## Next primitive

Do not spend another pass reshaping the same descriptor plan. The profile still implicates dense outer-join output assembly, but the failed micro-lever shows that extra parallel descriptor vectors increase memory traffic enough to lose.

Alien-graveyard routing for the next bead:

- `alien_cs_graveyard.md` section 8.14: Radix Hash Join / cache-shaped in-memory equi-join.
- `high_level_summary_of_frankensuite_planned_and_implemented_features_and_concepts.md`: vectorized execution and radix/cache-shaped join levers for join/operator hotspots.

Next target: a partitioned/radix dense int64 outer-join output path that batches descriptor emission and value-lane construction per cache-sized bucket range, with a target ratio of at least 1.25x on `outer_join 500000 200` while preserving exact pandas-observable ordering, duplicate tie-breaking, null placement, and golden SHA.
