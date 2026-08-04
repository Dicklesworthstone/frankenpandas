# br-frankenpandas-uza04.46 proof: incremental lower-hex UTF8 constructor

## Profile-backed target

Post-`3wo1d` reprofile (`tests/artifacts/perf/perf_report_after_3wo1d_lower_hex_seed.txt`)
showed ordered UTF8 setup had shifted into
`Column::from_lower_hex_sequence_utf8`:

- `Column::from_lower_hex_sequence_utf8`: 46.01% children, 32.39% self
- `__memmove_avx_unaligned_erms`: 48.72% children, 7.47% self

## One lever

Replace per-row fixed-width lower-hex formatting with one initialized row key
plus in-place ASCII lower-hex suffix incrementing. Bytes, offsets, all-valid
layout, fixed-width witness, strict-increasing witness, and
`Utf8LowerHexSequence` certificate construction stay in the same constructor.

## Isomorphism proof

- For every non-empty accepted input, the existing guard proves
  `start + len - 1` fits in `hex_width`; therefore every emitted value in the
  sequence fits in the fixed-width lowercase hexadecimal suffix.
- Row 0 is still emitted by `push_fixed_width_lower_hex(start, hex_width)`.
- Row `i + 1` is emitted by adding one to the previous ASCII lower-hex suffix.
  The carry rule maps `0..8 -> +1`, `9 -> a`, `a..e -> +1`, and `f -> 0`
  with carry to the next digit, which is exactly fixed-width base-16 addition.
- Prefix bytes are copied unchanged for every row.
- Offsets are still pushed immediately after each full row key, so every span
  remains `prefix || fixed_width_lower_hex(start + row)`.
- Output ordering and tie-breaking are unchanged because row order and key bytes
  are unchanged.
- Floating-point and RNG behavior are unchanged; this constructor emits only
  UTF8 bytes and offsets.
- Empty input remains accepted and yields an empty all-valid UTF8 column with no
  lower-hex certificate.

## Golden SHA256

Normal ordered UTF8 join golden:

- before: `2ac49173153820d4b3878817c44be31979faa18b2ae034167f7977adee83b02e`
- after:  `2ac49173153820d4b3878817c44be31979faa18b2ae034167f7977adee83b02e`

Empty ordered UTF8 join golden:

- before: `fc03a4635d1fe035e39a6f625acc9a3093dae0e9c61429a5a5c9742b146d0129`
- after:  `fc03a4635d1fe035e39a6f625acc9a3093dae0e9c61429a5a5c9742b146d0129`

`cmp -s` passed for both normal and empty before/after artifacts.

## Benchmarks

All commands were run through the `rch exec --` wrapper; workers failed
preflight and RCH fell open locally, so builds remained crate-scoped and target
dirs were isolated.

Baseline, full command, 1M rows, 20 internal merge iterations:

- before: `79.2 ms +/- 5.6`

Paired full command, 1M rows:

- before: `79.3 ms +/- 6.8`
- after:  `69.9 ms +/- 6.0`
- ratio:  `1.13x +/- 0.14`

Reversed paired full command, 1M rows:

- after:  `73.6 ms +/- 4.8`
- before: `78.2 ms +/- 4.2`
- ratio:  `1.06x +/- 0.09`

Paired full command, 2M rows:

- before: `154.9 ms +/- 6.3`
- after:  `144.7 ms +/- 5.2`
- ratio:  `1.07x +/- 0.06`

Setup-only target (`--iters 0 --warmup 0`), 1M rows:

- before: `77.0 ms +/- 4.7`
- after:  `68.9 ms +/- 5.8`
- ratio:  `1.12x +/- 0.12`
- user CPU: `34.1 ms -> 28.6 ms`

Score: Impact 2.7 x Confidence 0.9 / Effort 1.0 = 2.43. Keep.

## Reprofile

After reprofile on 5M-row setup-only target
(`tests/artifacts/perf/perf_report_after_uza0446_lower_hex_incremental.txt`):

- `__memmove_avx_unaligned_erms`: 32.86% self
- `Column::from_lower_hex_sequence_utf8`: 9.10% self

The constructor formatting bottleneck shifted to bulk byte movement / fixed-width
UTF8 storage materialization.

## Validation

- `cargo test -p fp-columnar lower_hex_sequence_constructor --lib -- --nocapture`
- `cargo test -p fp-join ordered_unique_utf8_lower_hex --lib -- --nocapture`
- `cargo check -p fp-columnar -p fp-join --all-targets`
- `cargo clippy -p fp-columnar -p fp-join --all-targets -- -D warnings`
- `cargo fmt -p fp-columnar -p fp-join -- --check`
- `ubs crates/fp-columnar/src/lib.rs`
