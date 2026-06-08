# br-frankenpandas-lr52z proof - sorted contiguous-Utf8 join seek bounds

## Target

- Scenario: `perf_profile str_inner_join 1000000 10`
- Hotspot before: `fp_join::merge_single_key_inner_unsorted`
- Profile-backed wall: ordered contiguous-Utf8 byte comparisons, including `__memcmp_avx2_movbe` and `strictly_increasing_utf8_key_spans`
- Alien primitive: Leapfrog-style sorted iterator seeking / binary-search lower bounds over ordered key streams

## Baseline

Command:

```text
rch exec -- hyperfine --warmup 2 --runs 10 --export-json tests/artifacts/perf/fp_before_str_inner_join_lr52z_orangepeak.json '/data/projects/.cargo-target/frankenpandas-orangepeak-lr52z/release-perf/examples/perf_profile str_inner_join 1000000 10'
```

Result:

```text
246.6 ms +/- 16.4 ms
```

Golden:

```text
76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e  tests/artifacts/perf/golden_before_str_inner_join_lr52z_orangepeak.txt
```

Before profile:

```text
merge_single_key_inner_unsorted: 69.94% children
__memcmp_avx2_movbe under join: 34.30% children
strictly_increasing_utf8_key_spans: 10.42% children
```

## Change

One lever only: keep the existing strict all-valid contiguous-Utf8 sorted/unique witness, then bound the ordered merge by sorted key overlap:

- early-empty when first/last spans prove disjoint ranges,
- binary-search lower bound on the left stream for the first right key,
- binary-search lower bound on the right stream for the first kept left key,
- continue the existing absolute-position two-cursor merge from those bounds.

## Isomorphism

- Ordering preserved: yes. The fast path still only runs when both key columns are strictly increasing under the same byte-slice ordering. Skipped rows are provably less than the first possible overlap key, so they cannot emit pairs. Emitted positions remain absolute left/right row indexes and are pushed in increasing left order.
- Tie-breaking unchanged: yes. Strictly increasing keys have no duplicate ties inside the fast path. Duplicate or unsorted inputs still return `None` from the witness and fall back to the existing left-major byte-span hash path.
- Floating-point: N/A for key matching. Value columns are gathered through the same output builder; no arithmetic changes.
- RNG: N/A.
- Null semantics: unchanged. `Column::as_utf8_contiguous()` still requires all-valid `LazyContiguousUtf8`; nullable or scalar-backed string keys fall back.
- Golden outputs: before and after SHA both equal `76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e`; `cmp -s` passed.

## After

Command:

```text
rch exec -- hyperfine --warmup 2 --runs 10 --export-json tests/artifacts/perf/fp_after_str_inner_join_lr52z_orangepeak.json '/data/projects/.cargo-target/frankenpandas-orangepeak-lr52z/release-perf/examples/perf_profile str_inner_join 1000000 10'
```

Result:

```text
208.2 ms +/- 5.8 ms
```

Speedup:

```text
1.1848x
```

Score:

```text
Impact 1.1848 x Confidence 0.95 / Effort 0.5 = 2.25
```

Effort is scored at 0.5 because the accepted lever is a small helper plus a single branch/test, with no data model migration.

After profile:

```text
merge_single_key_inner_unsorted: 80.85% children
__memcmp_avx2_movbe under join: 49.24% children
strictly_increasing_utf8_key_spans: 19.85% children
```

The residual wall is now the strict witness and byte-span comparisons themselves; route the next lever to cached or fused monotonic-unique witness metadata.

## Verification

- `cargo fmt -p fp-join --check`: pass
- `rch exec -- env CARGO_TARGET_DIR=/data/projects/.cargo-target/frankenpandas-orangepeak-lr52z cargo test -p fp-join`: pass, 105 tests
- `rch exec -- env CARGO_TARGET_DIR=/data/projects/.cargo-target/frankenpandas-orangepeak-lr52z cargo check -p fp-join --all-targets`: pass, remote `vmi1149989`
- `rch exec -- env CARGO_TARGET_DIR=/data/projects/.cargo-target/frankenpandas-orangepeak-lr52z cargo clippy -p fp-join --all-targets -- -D warnings`: pass, remote `vmi1167313`
- `ubs crates/fp-join/src/lib.rs`: nonzero due broad pre-existing scanner findings in this large file, including false-positive secret comparisons on `DType::Int64` checks and existing test equality; no new UBS-specific issue was identified for the seek-bound helper.
