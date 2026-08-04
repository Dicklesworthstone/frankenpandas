# br-frankenpandas-uza04.47 proof: lazy lower-hex UTF8 sequence storage

## Profile-backed target

Post-`uza04.46` reprofile
(`tests/artifacts/perf/perf_report_after_uza0446_lower_hex_incremental.txt`)
showed the formatter had shifted out of the hot path:

- `__memmove_avx_unaligned_erms`: 32.86% self
- `Column::from_lower_hex_sequence_utf8`: 9.10% self

That profile made eager fixed-width UTF8 byte/offset materialization the next
target.

## One lever

Add `ScalarValues::LazyLowerHexSequenceUtf8`, storing only:

- prefix bytes
- start value
- row count
- fixed lower-hex width
- cached materialized buffers, only if a caller asks for contiguous UTF8 spans

`Column::as_lower_hex_sequence_utf8()` exposes the exact sequence witness
without materializing bytes. `fp-join` consumes that witness for ordered
lower-hex overlap planning instead of calling the contiguous accessor.

## Isomorphism proof

- Constructor validation is unchanged: non-empty sequences still require
  `start + len - 1` to fit in `hex_width`, and empty sequences remain accepted.
- Materialized row `i` is still
  `prefix || fixed_width_lower_hex(start + i)`.
- `values()`, `as_utf8_contiguous()`, strict/fixed-width contiguous accessors,
  and `as_lower_hex_sequence_utf8_contiguous()` all materialize through the same
  incremental byte/offset builder used by the prior implementation, so any
  consumer that needs bytes observes the old layout exactly.
- The new direct witness returns the same prefix, start, width, and row count
  the contiguous path would derive, but does not allocate/copy key bytes.
- Ordered join output ordering and tie behavior are unchanged: the overlap math
  still computes the same contiguous `(left_start, right_start, len)` range from
  the same lower-hex values.
- Null layout is unchanged: the column is all-valid and uses the same
  `ValidityMask::all_valid(len)`.
- Floating-point and RNG behavior are unchanged; the lever only changes UTF8 key
  storage and witness access.

Focused laziness test:

- `lower_hex_sequence_witness_stays_lazy_uza0447` verifies the direct witness
  does not fill the materialized buffer cache, while the contiguous accessor
  fills it and returns the same certificate.

## Golden SHA256

Normal ordered UTF8 join golden:

- before: `2ac49173153820d4b3878817c44be31979faa18b2ae034167f7977adee83b02e`
- after:  `2ac49173153820d4b3878817c44be31979faa18b2ae034167f7977adee83b02e`

Empty ordered UTF8 join golden:

- before: `fc03a4635d1fe035e39a6f625acc9a3093dae0e9c61429a5a5c9742b146d0129`
- after:  `fc03a4635d1fe035e39a6f625acc9a3093dae0e9c61429a5a5c9742b146d0129`

`cmp -s` passed for both normal and empty before/after artifacts.

## Benchmarks

All build/benchmark commands were run through `rch exec --`; workers failed
preflight and RCH fell open locally, so target dirs were isolated and builds
remained crate-scoped.

Baseline setup-only target, current main:

- before: `69.3 ms +/- 2.8`

Paired setup-only target (`--iters 0 --warmup 0`), 1M rows:

- before: `69.5 ms +/- 5.9`
- after:  `22.5 ms +/- 1.5`
- ratio:  `3.09x +/- 0.33`
- user CPU: `28.8 ms -> 6.0 ms`
- system CPU: `40.3 ms -> 16.3 ms`

Paired full ordered UTF8 command, 1M rows / 20 internal merge iterations:

- before: `71.7 ms +/- 6.0`
- after:  `57.1 ms +/- 3.0`
- ratio:  `1.26x +/- 0.12`

Score: Impact 5.0 x Confidence 0.95 / Effort 1.0 = 4.75. Keep.

## Reprofile

After reprofile on 5M-row setup-only target
(`tests/artifacts/perf/perf_report_after_uza0447_lazy_lower_hex_storage.txt`):

- `__memmove_avx_unaligned_erms`: 24.69% self
- no named `Column::from_lower_hex_sequence_utf8` bucket in the top report

The next residual is general frame/value construction and remaining bulk moves,
not lower-hex sequence byte construction.

## Validation

- `cargo test -p fp-columnar lower_hex_sequence --lib -- --nocapture`
- `cargo test -p fp-join ordered_unique_utf8_lower_hex --lib -- --nocapture`
- `cargo test -p fp-columnar lower_hex_sequence_witness_stays_lazy_uza0447 --lib -- --nocapture`
- `cargo check -p fp-columnar -p fp-join --all-targets`
- `cargo clippy -p fp-columnar -p fp-join --all-targets -- -D warnings`
- `cargo fmt -p fp-columnar -p fp-join -- --check`
- `ubs crates/fp-columnar/src/lib.rs` exited 0 after removing the new test
  `panic!` surface.
- `ubs crates/fp-columnar/src/lib.rs crates/fp-join/src/lib.rs` exits 1 because
  the pre-existing `fp-join` file-wide scan reports false criticals on dtype and
  sentinel comparisons outside this patch (`dtype() != DType::Int64`,
  `lp != NONE_POS`). The `fp-join` diff here only swaps the lower-hex join plan
  from contiguous byte access to the direct witness accessor.
