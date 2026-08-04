# br-frankenpandas-jbyuc.1.1.1.1.1.1.1 Proof

## Target

After commit `2cdc737b`, `str_inner_join 1000000 5000` no longer sampled the ordered UTF8 `memcmp` proof. The next profile-backed residual was `std::thread::available_parallelism` under `fp_join::build_single_key_inner_merge_output_with_selections`, with cgroup quota open/read work visible in `perf_report_base_str_inner_join_jbyuc1111111.txt`.

## Lever

Resolve join parallelism once per process with `OnceLock<usize>`:

- Add `join_parallel_thread_count()` in `fp-join`.
- Keep the existing cap at `DENSE_I64_INNER_PARALLEL_MAX_CHUNKS`.
- Replace six direct `std::thread::available_parallelism()` call sites in join output builders.

This removes repeated `/sys/fs/cgroup` quota open/read probes from hot merge paths without changing chunk boundaries for a stable process quota.

## Isomorphism

- Ordering and tie-breaking: unchanged. The same output-position plans, spec order, and chunk partition formulas are used.
- Parallel determinism: unchanged. The helper returns the same capped thread count that each call computed before; it is only cached.
- Null/NaN/FP bits: unchanged. Column data construction and typed gather/slice logic are not modified.
- Suffix/name behavior: unchanged. Output spec resolution is not modified.
- RNG: not touched.
- Dynamic quota note: this intentionally snapshots the process join parallelism count. That affects worker count only if cgroup quota changes mid-process; pandas-observable values remain unchanged.
- Golden output SHA for `perf_profile golden str_inner_join 1000000`: `76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e` before and after.

## Benchmark

Command:

```text
hyperfine --warmup 2 --runs 10 '<perf_profile> str_inner_join 1000000 5000'
```

Baseline:

- Mean: `466.9 ms +/- 12.1 ms`
- Median: `465.3 ms`
- User/System: `123.6 ms / 342.2 ms`

After:

- Mean: `147.3 ms +/- 5.4 ms`
- Median: `148.2 ms`
- User/System: `74.0 ms / 72.7 ms`

Delta:

- Mean speedup: `3.17x`
- Mean reduction: `319.6 ms`
- System-time reduction: `269.5 ms`

## Profile Shift

Baseline no-children profile:

- `available_parallelism` under `build_single_key_inner_merge_output_with_selections`.
- Cgroup quota open/read path visible through `quota_v2`, `__libc_open64`, `read_to_string`, and `statx`.

After no-children profile:

- No `available_parallelism`, `quota_v2`, `__libc_open64`, or `read_to_string` hits in `perf_report_after_str_inner_join_jbyuc1111111.txt`.
- Top residual moved to `perf_profile::build_str_join_frame`, allocation/deallocation, and first-use column certificates.

## Validation

- `cargo fmt -p fp-join -- --check`
- `cargo test -p fp-join ordered_unique_utf8 --lib -- --nocapture`
- `cargo check -p fp-join --all-targets`
- `cargo clippy -p fp-join --all-targets -- -D warnings`
- `ubs crates/fp-join/src/lib.rs`

`rch` fell open locally for the scoped Cargo commands because all workers failed preflight checks.

UBS exited 0. It reported broad legacy heuristics, including false-positive secret-comparison findings on dtype/key equality; no actionable issue was introduced by this lever.

## Score

Impact `4` x Confidence `4` / Effort `1` = `16.0`, so this clears the required `>= 2.0` keep threshold.
