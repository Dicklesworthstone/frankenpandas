# br-frankenpandas-uza04.62 proof - Arc-backed Int64 source provenance

Agent: OrangePeak
Date: 2026-06-09
Commit target: `fp-columnar` Int64 cache/provenance

## Target

After br-frankenpandas-uza04.61, `perf_profile outer_join 500000 200`
showed the dense outer residual had shifted to source-copy pressure:

- Baseline profile: `__memmove_avx_unaligned_erms` 23.05%.
- Profile attribution under memmove: `Column::as_i64_arc` 16.10%.
- Root cause: scalar-constructed all-valid Int64 columns cached `ColumnData::Int64`
  as `Vec<i64>`, so dense outer source descriptors rebuilt `Arc<[i64]>` with
  `Arc::from(slice)` on every merge.

## Lever

One lever only:

- Change `ColumnData::Int64` from `Vec<i64>` to immutable `Arc<[i64]>`.
- Convert scalar-derived Int64 and categorical typed caches into `Arc<[i64]>`
  once at column construction.
- Make `Column::as_i64_arc` clone the cached Arc directly.
- Make Int64 clone preservation use `lazy_all_valid_int64_arc`.
- Add `scalar_int64_cache_arc_provenance_is_shared` to assert source and clone
  provenance are pointer-shared.

No join ordering, key walk, validity, dtype, Float64 promotion, RNG, or hash
behavior changed.

## Isomorphism proof

Golden command:

```bash
/data/projects/.scratch/cargo-target-orangepeak-uza0462-{base,after}/release-perf/examples/perf_profile golden outer_join 20000 | sha256sum
```

Baseline SHA:

```text
453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750
```

After SHA:

```text
453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750
```

Behavior witness:

- Same dense outer bucket order and plan construction.
- Same validity masks; missing Int64 promotion still materializes `Null(NaN)`.
- Same `i64 -> f64` cast sites for promoted lanes.
- Same key/value buffers and row order; only ownership changed from per-call
  Arc construction to cached Arc sharing.

## Benchmarks

RCH note: `rch exec` failed open because all workers failed preflight checks;
commands remained crate-scoped and used isolated target dirs.

Direct timing:

| Case | Baseline | After | Ratio |
| --- | ---: | ---: | ---: |
| `outer_join 500000 20` | 14.154 ms/iter | 13.796 ms/iter | 1.03x |
| `outer_join 500000 200` | 9.567 ms/iter | 8.361 ms/iter | 1.14x |

Paired hyperfine:

| Case | Baseline | After | Ratio |
| --- | ---: | ---: | ---: |
| `outer_join 500000 20` | 320.5 ms +/- 15.7 | 285.3 ms +/- 15.5 | 1.12x +/- 0.08 |
| `outer_join 500000 200` | 1.930 s +/- 0.032 | 1.614 s +/- 0.050 | 1.20x +/- 0.04 |

After profile:

- `Column::as_i64_arc` no longer appears under `__memmove_avx_unaligned_erms`.
- New top residual is dense outer CSR/plan construction:
  `build_single_key_dense_i64_outer_merge_output::{closure#2}` 20.89%.

## Validation

Passed:

```bash
cargo fmt -p fp-columnar -- --check
rch exec -- cargo test -p fp-columnar scalar_int64_cache_arc_provenance_is_shared --lib
rch exec -- cargo test -p fp-join merge_outer_dense_int64_duplicates_matches_generic_validated_route --lib
rch exec -- cargo check -p fp-columnar -p fp-join --all-targets
rch exec -- cargo clippy -p fp-columnar --all-targets --no-deps -- -D warnings
rch exec -- cargo clippy -p fp-join --lib --no-deps -- -D warnings
ubs crates/fp-columnar/src/lib.rs
```

UBS result: exit 0, zero critical findings. Remaining warnings are the
pre-existing file-wide inventory.

## Score

Impact 3 x Confidence 4 / Effort 2 = 6.0. Kept.

## Next residual

Profile after this lever shifted to dense outer CSR/plan construction. Do not
repeat .60/.61/.62. The next candidate should attack a deeper compact
descriptor primitive that avoids generic CSR/plan construction for periodic
dense outer shapes while preserving the exact bucket walk and lane validity
semantics.
