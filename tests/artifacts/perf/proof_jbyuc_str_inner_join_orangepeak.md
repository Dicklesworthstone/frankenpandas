# br-frankenpandas-jbyuc proof: direct contiguous-UTF8 join key encoder

Agent: OrangePeak
Date: 2026-06-08
Target: `perf_profile str_inner_join 1000000`
Lever kept: replace `write!(&mut String, "id_{:08x}", key)` in `build_str_join_frame` with direct byte emission into the contiguous-UTF8 fixture buffer.

## Profile-backed target

Baseline profile after br-frankenpandas-483i5 showed the measured `str_inner_join`
run was still contaminated by string fixture construction:

- `<fp_columnar::Column>::take_positions`: 18.39%
- `perf_profile::build_str_join_frame`: 11.82%
- `core::fmt` / lower-hex formatting under `build_str_join_frame`: visible in the frame-construction stack
- `__memcmp_avx2_movbe`: 8.49%

The first attempted lever was a `Column::take_positions` unit-range fast path.
It was reverted because same-context A/B was only 1.01x:

- pre-lever: 141.735659693 ms +/- 3.519897554 ms
- attempted lever: 140.061315227 ms +/- 4.727234488 ms
- result: 1.01x, below keep threshold

The deeper primitive chosen from the alien-graveyard pass was zero-copy/batched
byte construction: emit the known ASCII key representation directly into the
columnar UTF8 bytes buffer instead of formatting through `core::fmt` and an
intermediate `String`.

## Benchmarks

Baseline via `rch exec -- hyperfine --warmup 3 --runs 10`:

- command: `/data/projects/.cargo-target/frankenpandas-orangepeak-jbyuc/release-perf/examples/perf_profile str_inner_join 1000000 10`
- mean: 153.520220220 ms
- stddev: 11.039252293 ms

After final helper via `rch exec -- hyperfine --warmup 3 --runs 10`:

- command: `/data/projects/.cargo-target/frankenpandas-orangepeak-jbyuc/release-perf/examples/perf_profile str_inner_join 1000000 10`
- mean: 112.248350700 ms
- stddev: 13.415010865 ms
- speedup vs baseline: 1.37x

Direct paired A/B via `rch exec -- hyperfine --warmup 3 --runs 10`:

- pre-lever binary: 145.328563940 ms +/- 5.937500180 ms
- after binary: 106.653426940 ms +/- 3.331370950 ms
- speedup: 1.36x +/- 0.07

Score:

- Impact: 1.36
- Confidence: 0.95 (same scenario, paired A/B, same golden output)
- Effort: 0.50
- Impact x Confidence / Effort = 2.58, keep

## Golden output

Golden command:

```text
/data/projects/.cargo-target/frankenpandas-orangepeak-jbyuc/release-perf/examples/perf_profile golden str_inner_join 1000000
```

SHA256 before:

```text
76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e
```

SHA256 after:

```text
76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e
```

`cmp -s` passed.

## Isomorphism proof

- The old generator emitted `id_` followed by `{:08x}` for `(row % card) + key_start`.
- The new generator emits the same `id_` prefix and computes the same lower-hex digits from most significant nibble to least significant nibble.
- Width semantics are preserved: values with fewer than 8 hex digits receive leading zero bytes; values with 8 or more hex digits receive no truncation and no extra padding, matching Rust formatting for `{:08x}`.
- Row order is unchanged: the loop still visits `0..n`, uses the same `(row % card) + key_start` expression, and pushes one offset immediately after each key.
- Join ordering, suffix/tie semantics, index labels, and value column construction are unchanged.
- No floating-point arithmetic, RNG, NaN/null propagation, or hash ordering changes are introduced by this lever.

## Validation

- `cargo fmt -p fp-conformance --check`: pass
- `rch exec -- env CARGO_TARGET_DIR=/data/projects/.cargo-target/frankenpandas-orangepeak-jbyuc cargo check -p fp-conformance --all-targets`: pass
- `rch exec -- env CARGO_TARGET_DIR=/data/projects/.cargo-target/frankenpandas-orangepeak-jbyuc cargo test -p fp-conformance --example perf_profile`: pass, 0 tests in example harness
- `rch exec -- env CARGO_TARGET_DIR=/data/projects/.cargo-target/frankenpandas-orangepeak-jbyuc cargo clippy -p fp-conformance --all-targets -- -D warnings`: blocked by pre-existing `fp-frame` `clippy::question_mark` lint in `crates/fp-frame/src/lib.rs:48256`
- `rch exec -- env CARGO_TARGET_DIR=/data/projects/.cargo-target/frankenpandas-orangepeak-jbyuc cargo clippy -p fp-conformance --example perf_profile --no-deps -- -D warnings`: pass
- `ubs crates/fp-conformance/examples/perf_profile.rs`: exit 0; no critical issues

## Reprofile after keep

Post-lever `perf record -m 8 -F 199 -g`:

- `perf_profile::build_str_join_frame`: still visible as fixture-construction cost, but the `core::fmt` lower-hex stack is gone from the top report.
- Next library-side target: `__memmove_avx_unaligned_erms` under `fp_join::build_single_key_inner_merge_output` / `merge_single_key_inner_unsorted` at 16.63%.

Follow-up should target batched/fused contiguous-UTF8 join output materialization, not another fixture encoder tweak.
