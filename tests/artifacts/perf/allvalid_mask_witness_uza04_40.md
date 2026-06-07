# br-frankenpandas-uza04.40 all-valid mask sentinel witness

Bead: `br-frankenpandas-uza04.40`

Lever: represent `ValidityMask::all_valid(len)` as a zero-word sentinel plus
logical length, materializing only if a caller clears a bit.

Selected primitive: succinct uniform bitvector / deferred materialization. The
profile-backed target was dense inner-join output assembly after lazy
repeat-run/repeated-slice lanes; the baseline profile showed
`__memset_avx2_unaligned_erms` at `23.36%` children, consistent with
full all-valid validity bitmap zero/fill cost for large lazy output columns.

## Passes

1. Baseline/profile: pass 1 established `ts1` RCH build at
   `6b2d4769f2f4c81de1016e9ba8eaac929b516743`, golden hashes, 10-run
   baseline timings, and CPU profile.
2. Alien primitive selection: pass 2 selected the all-valid validity sentinel
   over dense segment sharing and join algorithm replacement because the
   measured residual was output lifecycle/bitmap fill, not key discovery.
3. Single lever: changed only `ValidityMask` storage/operations in
   `crates/fp-columnar/src/lib.rs`; no join ordering code changed.
4. Proof/score: final paired benchmarks and golden SHA verification cleared
   the keep gate.
5. Reprofile: final after profile no longer has `__memset` in the visible top
   symbols; residual moved to dense output assembly plus `Column` new/drop.

## Behavior Proof

- Ordering and duplicate tie-breaking: join row construction code is unchanged.
  The lever only changes the representation of an all-valid mask attached to
  output columns.
- Floating point: no arithmetic or f64 comparison path changed.
- RNG: no RNG code touched or introduced.
- Null/NaN: `from_values` and `from_f64` still mark the same missing slots.
  All-valid inputs now store the same logical bit sequence as the prior all-ones
  bitmap. Clearing a bit materializes the explicit bitmap before mutation.
- Mask algebra: added tests prove sentinel behavior for equality against
  explicit all-ones words, mutation materialization, `and`, `or`, `xor`, `not`,
  `slice`, and `concat`.
- Serialization: `Serialize` still emits the backward-compatible `bits` field;
  `Deserialize` routes through `from_words`, which preserves logical bits and
  collapses all-valid bitstreams to the sentinel.
- Golden SHA: final after hashes equal pass 1 baseline:

```text
inner_join OK expected=494106fca6e3310a318f1685c74041a2788089a4d2409107d4eef4a00c7a0764 actual=494106fca6e3310a318f1685c74041a2788089a4d2409107d4eef4a00c7a0764
join_1to1 OK expected=102690aa39952cc2d13bcc41547aacdeac1946113e43d62472fdb93440bc56a7 actual=102690aa39952cc2d13bcc41547aacdeac1946113e43d62472fdb93440bc56a7
```

## Benchmarks

Final paired panel, RCH-built before/after binaries:

| Scenario | Before | After | Ratio |
| --- | ---: | ---: | ---: |
| `inner_join 100000 3` | `52.3 ms +/- 5.7` | `37.6 ms +/- 3.1` | `1.39x` |
| `join_1to1 100000 20` | `40.5 ms +/- 8.4` | `38.0 ms +/- 2.8` | `1.07x` |
| `inner_join_read 100000 3` | `370.0 ms +/- 16.9` | `389.1 ms +/- 62.6` | noisy outlier panel |

Final isolated control panels:

| Scenario | Before | After | Ratio |
| --- | ---: | ---: | ---: |
| `join_1to1 100000 20` | `36.8 ms +/- 3.1` | `34.7 ms +/- 2.5` | `1.06x` |
| `inner_join_read 100000 3`, reverse order | `372.5 ms +/- 12.7` | `368.4 ms +/- 15.1` | `1.01x` |

Score: Impact 4 x Confidence 4 / Effort 2 = `8.0`; retained.

## Validation

- `cargo fmt -p fp-columnar -- --check`
- RCH `cargo test -p fp-columnar validity -- --nocapture`
- RCH `cargo test -p fp-columnar repeated_slice_int64_column_matches_eager_materialization -- --nocapture`
- RCH `cargo test -p fp-join dense_i64_inner -- --nocapture`
- RCH `cargo check -p fp-columnar --all-targets`
- RCH `cargo clippy -p fp-columnar --all-targets -- -D warnings`
- `ubs crates/fp-columnar/src/lib.rs`: no critical findings; remaining warnings
  are broad pre-existing `fp-columnar` inventories. Formatter and clippy were
  clean inside UBS.

## Reprofile Route

Baseline visible profile included `__memset_avx2_unaligned_erms` at `23.36%`
children. Final after profile did not show `memset` above the report threshold;
visible residuals were:

- `fp_join::build_dense_i64_inner_output_data`: `28.05%` children / `8.30%`
  self
- `core::ptr::drop_in_place::<fp_columnar::Column>`: `11.26%` children /
  `2.82%` self
- `fp_join::build_single_key_dense_i64_inner_merge_output`: `8.44%` children /
  `5.63%` self
- `<fp_columnar::Column>::new`: `6.24%` self

Next primitive: attack shared dense join segment descriptors / output-column
lifecycle reuse so `build_dense_i64_inner_output_data` and `Column` drop/new
residuals move, while preserving the same row-order and golden-SHA contracts.
