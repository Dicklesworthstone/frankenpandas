# br-frankenpandas-dense-join-no-wasted-parallelism-probe-ikphf rejection witness

Bead: `br-frankenpandas-dense-join-no-wasted-parallelism-probe-ikphf`

Lever tested: guard the `std::thread::available_parallelism()` query inside
`fp_join::build_dense_i64_inner_output_data` so it only runs when dense full
lanes exist and the output length reaches the parallel-fill threshold.

Profile source: `tests/artifacts/perf/perf_report_dense_join_shared_plan_after_l4adm_inner_join.txt`
reported `fp_join::build_dense_i64_inner_output_data` at `52.48%` children /
`34.09%` self, with `std::thread::functions::available_parallelism` visible at
`4.57%` children.

## Behavior Proof

- Ordering preserved: yes. The candidate did not change `matched` construction,
  left probe order, right-bucket replay order, or output lane assembly.
- Tie-breaking unchanged: yes. Duplicate right-key order still came from
  `plan.positions[start..start + run_len]`.
- Floating point: N/A. The target path is all-valid `Int64`.
- RNG: N/A. No RNG path touched or introduced.
- Null/NaN: N/A for this all-valid dense `Int64` path.
- Golden SHA unchanged:

```text
inner_join before=be7d17114e5a2a88607bdc65228751789e208ebbd9ac2f5d0c616cdf64e641f1 after=be7d17114e5a2a88607bdc65228751789e208ebbd9ac2f5d0c616cdf64e641f1
join_1to1 before=18988b22befdb71941112638f6f1c3415cbf0f79fc1f00ecd013318d924682c7 after=18988b22befdb71941112638f6f1c3415cbf0f79fc1f00ecd013318d924682c7
```

## Benchmarks

RCH before build:

```text
env CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-parallelism-probe-ikphf-before-20260608T032916Z RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

RCH after build:

```text
env CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-parallelism-probe-ikphf-after-20260608T033740Z RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Paired local hyperfine over the two RCH-built binaries:

| Scenario | Before | After | Verdict |
| --- | ---: | ---: | --- |
| `inner_join 100000 50` | `116.8 ms +/- 10.9` | `302.9 ms +/- 13.8` | rejected, `0.39x` |
| `inner_join_read 100000 10` | `1.105 s +/- 0.016` | `1.145 s +/- 0.064` | rejected, neutral/slower |
| `join_1to1 100000 100` | `60.7 ms +/- 5.2` | `67.8 ms +/- 4.9` | rejected, `0.90x` |

Score: Impact 0 x Confidence 5 / Effort 1 = `0.0`; rejected. The source edit
was reverted and no candidate code remains.

## Route

This was a micro-lever against a visible but shallow system-call child frame.
The next attack should move back to a structural dense-output primitive: reduce
or replace the remaining `matched: Vec<(left_pos, bucket_start, run_len)>`
planner work in `build_dense_i64_inner_output_data` with a cache-shaped shared
join row plan / descriptor, preserving the existing left-major and right-bucket
ordering witness.
