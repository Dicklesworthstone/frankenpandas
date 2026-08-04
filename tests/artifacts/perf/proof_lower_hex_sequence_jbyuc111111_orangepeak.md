# br-frankenpandas-jbyuc.1.1.1.1.1.1 Proof

## Target

Ordered unique fixed-width UTF8 inner join profile gap after the prior contiguous-range work. The baseline profile showed `__memcmp_avx2_movbe` under `ordered_unique_utf8_inner_position_plan` as the top userspace residual for `str_inner_join 1000000 5000`.

## Lever

Add a deterministic lower-hex sequence certificate for immutable contiguous UTF8 columns and reuse the existing `ContiguousRanges` join plan when both sides prove the same fixed-width `prefix + lowercase_hex(start + row)` shape.

This is one lever:

- `fp-columnar` caches `Utf8LowerHexSequence` on `LazyContiguousUtf8`.
- `fp-join` consumes the certificate only inside the existing ordered-unique fixed-width UTF8 path.
- All uncertified inputs fall back to the existing byte-window equality and gather paths.

## Isomorphism

- Ordering and tie-breaking: unchanged. The optimized path returns the existing left-major `ContiguousRanges` plan only for the same overlap positions previously accepted by the byte-equality proof.
- Duplicate and gap behavior: unchanged. The certificate requires strict `start + row` arithmetic and is reached only after the existing strict-increasing and fixed-width checks.
- Null behavior: unchanged. The accessor is only exposed for all-valid `Utf8` `LazyContiguousUtf8`; nullable inputs fall back.
- String equality: preserved. The join reaches the certificate path only after the current left/right spans compare byte-equal; matching certificates then prove per-column constant prefixes, suffix width, lowercase hex encoding, and equal arithmetic progression for the rest of the overlap.
- Floating point and RNG: not touched.
- Golden output SHA for `perf_profile golden str_inner_join 1000000`: `76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e` before and after.

## Benchmark

Command:

```text
hyperfine --warmup 2 --runs 10 '<perf_profile> str_inner_join 1000000 5000'
```

Baseline:

- Mean: `562.5 ms +/- 8.1 ms`
- Median: `560.5 ms`
- User/System: `218.1 ms / 343.1 ms`

After:

- Mean: `456.6 ms +/- 8.8 ms`
- Median: `456.9 ms`
- User/System: `124.7 ms / 331.2 ms`

Delta:

- Mean speedup: `1.23x`
- Mean reduction: `105.9 ms`
- User-time reduction: `93.4 ms`

## Profile Shift

Baseline no-children profile:

- `__memcmp_avx2_movbe`: `13.34%` self under `ordered_unique_utf8_inner_position_plan`.

After no-children profile:

- No `__memcmp` samples in `perf_report_after_str_inner_join_jbyuc111111.txt`.
- Top residual moved into output construction and system/kernel file-open/cgroup accounting around `build_single_key_inner_merge_output_with_selections`.

## Validation

- `cargo fmt -p fp-columnar -p fp-join -- --check`
- `cargo test -p fp-columnar contiguous_utf8_lower_hex_sequence_certificate_jbyuc111111 --lib -- --nocapture`
- `cargo test -p fp-join ordered_unique_utf8 --lib -- --nocapture`
- `cargo check -p fp-columnar -p fp-join --all-targets`
- `cargo clippy -p fp-columnar -p fp-join --all-targets -- -D warnings`
- `ubs crates/fp-columnar/src/lib.rs crates/fp-join/src/lib.rs`

`rch` fell open locally for the scoped Cargo commands because all workers failed preflight checks.

UBS exited 0. It reported broad legacy heuristics on the two large files, including false-positive secret-comparison findings on dtype/key equality; no actionable issue was introduced by this lever.

## Score

Impact `3` x Confidence `4` / Effort `2` = `6.0`, so this clears the required `>= 2.0` keep threshold.
