# br-frankenpandas-uza04.145 df_dot phase profile

Agent: LavenderStone
Target: `df_dot 100000`
Decision: profiling evidence only; no runtime source retained

## Why this pass existed

`br-frankenpandas-uza04.144` rejected a per-column chunk fanout route for
`df_dot`. `df_dot` remained the largest current-main target in the fresh routing
matrix, but Linux perf and samply are blocked on this host by
`perf_event_paranoid=4`. This pass temporarily added env-gated phase timers to
`DataFrame::dot`, built an instrumented release-perf binary, measured the
current path, and removed the timers before commit.

## Phase profile

Command:

`FP_DOT_PROFILE=1 perf_profile df_dot 100000 3`

Artifact:

- `tests/artifacts/perf/lavender_uza04145_dot_phase_profile_100000x3.txt`

Per-iteration `DataFrame::dot` phases:

| Iter | extract_a | copy_b | pack_b | compute_bands | build_columns | assemble_map | dot total |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 29.877 us | 40.667 us | 58.831 us | 70.888 ms | 3.352 ms | 99.368 us | 74.532 ms |
| 2 | 45.557 us | 111.772 us | 162.117 us | 57.787 ms | 2.817 ms | 97.073 us | 61.068 ms |
| 3 | 49.905 us | 141.528 us | 142.119 us | 60.258 ms | 3.129 ms | 84.971 us | 63.859 ms |

The harness reported `156.330 ms/iter` for the same run. The gap is outside the
timed `dot()` body and is consistent with result destruction/freeing after each
iteration: every benchmark iteration drops the returned ~205MB result frame.

## Interpretation

- Current `dot()` internals are dominated by `compute_bands` (`~58-71ms`).
- `build_columns` is no longer a large in-body residual (`~3ms`).
- A route that only rearranges final column chunk fanout is unlikely to win; the
  `.144` candidate confirmed this by increasing wall and system time.
- The largest remaining benchmark wall cost includes allocation/free churn
  outside the `dot()` body. A deeper next primitive should avoid producing a
  fully materialized result when consumers do not read values, or should attack
  compute without adding allocation pressure.

## Next route

Attack a lazy dot-output primitive: return behavior-identical lazy Float64
columns that hold the dot operands and output-column index, compute values on
materialization/access in the same `l=0..k` order, and preserve golden SHA when
forced. This targets the observed benchmark shape (`out.len()` only) and keeps
full value parity for consumers that read the result.

The alternative is a compute-only primitive, but phase data shows that would
need to improve the `~58-71ms` compute phase without making the external
allocation/drop cost worse.

## Cleanup

The temporary `FP_DOT_PROFILE` source hunk was removed before this evidence
commit. No runtime source is retained for `.145`.
