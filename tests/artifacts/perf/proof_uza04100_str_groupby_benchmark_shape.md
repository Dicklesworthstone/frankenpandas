# br-frankenpandas-uza04.100 string groupby benchmark-shape correction

## Target

`br-frankenpandas-uza04.96` found that the timed `str_groupby_*` scenarios used
`build_str_key_frame`, whose key text included the full per-row mixed value:

```text
key_{mixed:016x}_{key_id:04x}
```

That made the timed scenarios effectively all-distinct even when a bounded
cardinality was requested. This commit changes only the timed `str_groupby_*`
scenarios to use the existing `build_str_key_frame_repeated(n, 64)` fixture so
the harness profiles the realistic K << n branch before product work.

Product code is unchanged.

## Baseline

Build:

```text
CARGO_TARGET_DIR=.rch-target-uza04100-base
RUSTFLAGS='-C force-frame-pointers=yes'
rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

RCH failed open locally (`no admissible workers: insufficient_slots=2,
hard_preflight=10`), but the build was crate-scoped to `fp-conformance`.

Kernel profiling was blocked:

```text
perf_stat_status=255
perf_event_paranoid setting is 4
```

Baseline timed shape:

```text
str_groupby_count 200000 6:       202.8 ms +/- 18.0 ms
str_groupby_sum 200000 6:         193.4 ms +/-  6.6 ms
str_groupby_std 200000 6:         253.2 ms +/- 21.1 ms
str_groupby_count_lowcard 200000 20: 63.6 ms +/- 3.0 ms
str_groupby_sum_lowcard 200000 20:   60.6 ms +/- 2.3 ms
```

## Golden Proof

Before and after SHA256 rows matched exactly:

```text
77a2baa488f3b39462a15e4cc2223a884afb0d48e088b2ed14b0c2138f8a1894  str_groupby_count 1000
d5cea8c5844bb962c8958f0a94d53a9e7d6b452a55ccaa51c5addb6000c0c46e  str_groupby_count 200000
a53b6ca20edea8eafabefe76ee5c12bd98d116d02d6372a93707fdc258f1c80d  str_groupby_sum 1000
a28f5534bd1a01e792695b5f80aa877724803caff9bbed0b6fc86807e20b5412  str_groupby_sum 200000
33cf06ed6e0f6604c0fb7fb7c91f029e53cf80570c5a987ad4f4382d2e561343  str_groupby_std 1000
72e4b0378ad8409249711f58264e5c4cd64109f5c6f9d684d5e2c419da22a5f5  str_groupby_std 200000
```

`tests/artifacts/perf/uza04100_shape_golden_cmp.txt` records `cmp=0` for all
six before/after golden files.

Isomorphism:

- Product behavior: unchanged; no library code changed.
- Ordering/tie-breaking/null/NaN/f64/RNG: unchanged for all product paths.
- Harness timed shape: intentionally changed from the accidental all-distinct
  fixture to the existing repeated-key fixture so the named benchmark exercises
  the same realistic grouping branch as the explicit low-cardinality scenarios.

## Benchmark

Paired hyperfine:

```text
str_groupby_count 200000 6: 204.5 ms +/-  6.1 -> 31.9 ms +/- 2.5 (6.41x)
str_groupby_sum 200000 6:   184.8 ms +/- 12.0 -> 33.4 ms +/- 2.8 (5.53x)
str_groupby_std 200000 6:   226.6 ms +/-  9.7 -> 36.0 ms +/- 3.1 (6.29x)
```

This is a measurement-shape correction, not a product-speed claim. Score for
the harness lever: Impact 3 x Confidence 5 / Effort 1 = 15.0, kept because it
unblocks the intended profile target and removes the accidental all-distinct
benchmark artifact.

## Validation

- `rch exec -- cargo check -p fp-conformance --example perf_profile` passed on
  `vmi1153651`.
- `ubs crates/fp-conformance/examples/perf_profile.rs` reported zero critical
  findings; remaining warning/info inventory is pre-existing in the benchmark
  harness.
- `cargo fmt -p fp-conformance --check` and direct `rustfmt --check` are blocked
  by pre-existing formatting drift in unrelated sections/examples, so this
  commit does not reformat unrelated code.

## Next Profile Route

The corrected named scenarios now measure the low-cardinality dense string
groupby branch. The next product pass should profile that branch and attack a
real library hotspot with one lever, likely fusing grouping with simple
accumulators or another dictionary-coded key-interning primitive if the corrected
profile supports it.
