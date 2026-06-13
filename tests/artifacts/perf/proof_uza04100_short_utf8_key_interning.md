# br-frankenpandas-uza04.100 short UTF-8 key interning proof

## Target

Profile-backed product lever after the `.100` benchmark-shape correction:
the corrected low-cardinality string groupby path spends material time
hashing byte slices while assigning dense group ids before sorting the
distinct keys.

## Lever

For the `sort=true`, low-cardinality string groupby branch only, detect
fixed-width UTF-8 key spans of at most 8 bytes and intern them through a
packed `u64` key map instead of a borrowed byte-slice map. All other key
shapes use the existing `FxHashMap<&[u8], usize>` path.

## Behavior proof

- Group equality is unchanged. The packed path is entered only when every
  key span has the same byte length and that length is <= 8, so
  `pack_short_utf8_span` is injective over the observed key domain.
- Output ordering is unchanged. The packed map only assigns first-seen dense
  group ids; output order is still produced by `utf8_msd_argsort_bytes` over
  the representative original byte spans, followed by the existing gid remap.
- Tie-breaking is unchanged. For a duplicated key, the representative row is
  still the first row where that exact key is seen, matching the previous
  low-cardinality branch.
- Floating-point behavior is unchanged. Aggregators still consume the same
  rows into the same dense groups; only string-key interning changes.
- RNG behavior is unchanged. The deterministic benchmark fixtures and
  groupby implementation do not add or remove any random source.
- Fallback behavior is unchanged for non-fixed-width keys and fixed-width keys
  wider than 8 bytes.

## Golden output SHA-256

All candidate goldens match the current-main benchmark-shape baseline:

```text
str_groupby_count 1000 cmp=0
str_groupby_count 200000 cmp=0
str_groupby_sum 1000 cmp=0
str_groupby_sum 200000 cmp=0
str_groupby_std 1000 cmp=0
str_groupby_std 200000 cmp=0
```

Candidate hashes:

```text
77a2baa488f3b39462a15e4cc2223a884afb0d48e088b2ed14b0c2138f8a1894  tests/artifacts/perf/uza04100_after_product_golden_str_groupby_count_1000.txt
d5cea8c5844bb962c8958f0a94d53a9e7d6b452a55ccaa51c5addb6000c0c46e  tests/artifacts/perf/uza04100_after_product_golden_str_groupby_count_200000.txt
33cf06ed6e0f6604c0fb7fb7c91f029e53cf80570c5a987ad4f4382d2e561343  tests/artifacts/perf/uza04100_after_product_golden_str_groupby_std_1000.txt
72e4b0378ad8409249711f58264e5c4cd64109f5c6f9d684d5e2c419da22a5f5  tests/artifacts/perf/uza04100_after_product_golden_str_groupby_std_200000.txt
a53b6ca20edea8eafabefe76ee5c12bd98d116d02d6372a93707fdc258f1c80d  tests/artifacts/perf/uza04100_after_product_golden_str_groupby_sum_1000.txt
a28f5534bd1a01e792695b5f80aa877724803caff9bbed0b6fc86807e20b5412  tests/artifacts/perf/uza04100_after_product_golden_str_groupby_sum_200000.txt
```

## Paired benchmark

Baseline binary: `.rch-target-uza04100-after-shape/release-perf/examples/perf_profile`
Candidate binary: `.rch-target-uza04100-after-product/release-perf/examples/perf_profile`

`hyperfine --warmup 3 --runs 10`, 200000 rows, 20 harness iterations:

| Scenario | Before mean | After mean | Speedup |
| --- | ---: | ---: | ---: |
| `str_groupby_count` | 62.1 ms +/- 4.1 ms | 55.9 ms +/- 3.1 ms | 1.11x |
| `str_groupby_sum` | 66.1 ms +/- 5.1 ms | 50.1 ms +/- 2.5 ms | 1.32x |
| `str_groupby_std` | 68.3 ms +/- 4.3 ms | 57.2 ms +/- 2.6 ms | 1.19x |

Score: Impact 2.4 x Confidence 4.0 / Effort 1.5 = 6.4, so keep.

## Validation

- `CARGO_TARGET_DIR=.rch-target-uza04100-after-product RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
  completed; RCH failed open locally because no worker slots were admissible.
- `CARGO_TARGET_DIR=.rch-target-uza04100-check-frame rch exec -- cargo check -p fp-frame --all-targets`
  passed on worker `vmi1153651`.
- `CARGO_TARGET_DIR=.rch-target-uza04100-clippy-frame-lib rch exec -- cargo clippy -p fp-frame --lib -- -D warnings`
  passed on worker `vmi1153651`.
- `CARGO_TARGET_DIR=.rch-target-uza04100-test-frame rch exec -- cargo test -p fp-frame groupby_sum_string_column_concatenates_6lnll --lib`
  passed on worker `vmi1153651`.
- `git diff --check` passed.
- `cargo clippy -p fp-frame --all-targets -- -D warnings` remains blocked by
  pre-existing test-only `clippy::type_complexity` and `clippy::useless_vec`
  findings around `crates/fp-frame/src/lib.rs:83219`, `83410`, `83702`,
  `83918`, `84034`, `84172`, and `84542`.
- `rustfmt --edition 2024 --check crates/fp-frame/src/lib.rs` remains blocked
  by pre-existing formatting drift at `crates/fp-frame/src/lib.rs:41753`.
- `ubs crates/fp-frame/src/lib.rs` timed out after 90 seconds without emitting
  findings.

## Re-profile note

`perf stat -e cycles,instructions,cache-misses` is blocked on this host by
`perf_event_paranoid=4` (`perf_stat_exit=255`). The follow-up route should use
the corrected low-cardinality benchmark plus any available external profiler
on a host with perf access; do not reuse the rejected all-singleton path.
