# br-frankenpandas-l4adm dense join shared descriptor witness

Bead: `br-frankenpandas-l4adm`

Lever: dense i64 inner-join lazy lanes now share immutable run-length and
repeated-slice descriptors via `Arc<[_]>`. Left repeat-run lanes store only
their per-run values plus a shared run-length vector; right repeated-slice lanes
store their value tape plus a shared `(start, len)` segment vector.

Selected primitive: vectorized execution selection-vector / RLE descriptor
sharing. This follows the post-`br-frankenpandas-uza04.40` profile where
`__memset` moved out and the residual shifted to
`fp_join::build_dense_i64_inner_output_data` plus `Column` construction/drop.

## Behavior Proof

- Ordering preserved: yes. `matched` is still built in left probe order, and
  right buckets still replay `plan.positions[start..start + run_len]`.
- Tie-breaking unchanged: yes. Duplicate-key right order remains CSR bucket
  insertion order; left-major/right-minor row order is unchanged.
- Floating point: N/A. This dense path is all-valid `Int64` only.
- RNG: N/A. No RNG path touched or introduced.
- Null/NaN: N/A for this all-valid dense `Int64` path; validity remains
  `ValidityMask::all_valid(output_len)`.
- Golden SHA unchanged:

```text
inner_join before=494106fca6e3310a318f1685c74041a2788089a4d2409107d4eef4a00c7a0764 after=494106fca6e3310a318f1685c74041a2788089a4d2409107d4eef4a00c7a0764
join_1to1 before=102690aa39952cc2d13bcc41547aacdeac1946113e43d62472fdb93440bc56a7 after=102690aa39952cc2d13bcc41547aacdeac1946113e43d62472fdb93440bc56a7
```

## Benchmarks

Both binaries were RCH-built on `ts1`:

- Before binary: `/data/projects/.scratch/cargo-target-orangepeak-l4adm-baseline-20260607T1857Z/release-perf/examples/perf_profile`
- After binary: `/data/projects/.scratch/cargo-target-orangepeak-l4adm-after-20260607T1909Z/release-perf/examples/perf_profile`

Paired local hyperfine over the two RCH-built binaries:

| Scenario | Before | After | Ratio |
| --- | ---: | ---: | ---: |
| `inner_join 100000 50` | `299.4 ms +/- 14.4` | `113.8 ms +/- 11.2` | `2.63x` |
| `inner_join_read 100000 10` | `1.210 s +/- 0.039` | `1.202 s +/- 0.097` | `1.01x` |
| `join_1to1 100000 100` | `63.9 ms +/- 8.5` | `63.1 ms +/- 5.2` | `1.01x` |

Perf-stat control for `inner_join 100000 50`:

| Counter | Before | After | Ratio |
| --- | ---: | ---: | ---: |
| cycles | `1,036,596,301` | `569,075,207` | `1.82x` |
| instructions | `1,261,602,301` | `765,937,055` | `1.65x` |
| cache references | `106,861,485` | `76,269,835` | `1.40x` |
| cache misses | `8,333,898` | `3,690,951` | `2.26x` |

Score: Impact 4 x Confidence 5 / Effort 2 = `10.0`; retained.

## Validation

- `cargo fmt -p fp-columnar -p fp-join -- --check`
- RCH `cargo test -p fp-columnar repeat -- --nocapture`
- RCH `cargo test -p fp-join dense_i64_inner -- --nocapture`
- RCH `cargo check -p fp-columnar -p fp-join --all-targets`
- RCH `cargo clippy -p fp-columnar -p fp-join --all-targets -- -D warnings`
- `ubs crates/fp-columnar/src/lib.rs crates/fp-join/src/lib.rs`

UBS result: nonzero due pre-existing false-positive criticals in
`fp-join/src/lib.rs` (`DType::Int64` equality and test integer-key equality
flagged as secret comparisons). `git blame` shows those lines predate this
lever (`56b5d44c3`, `6e5df6f1a`) and they are not secrets/tokens. Formatter,
clippy, cargo check, focused tests, and golden outputs are clean.

## Reprofile Route

Before profile: `build_dense_i64_inner_output_data` was `69.17%` children /
`18.20%` self, and `core::ptr::drop_in_place::<fp_columnar::Column>` was
`8.15%`.

After profile: `build_dense_i64_inner_output_data` is still the visible hot
cluster (`52.48%` children / `34.09%` self), but total command time and
cache-miss counts dropped substantially. The next profile-backed primitive
should attack the remaining dense output planner work inside
`build_dense_i64_inner_output_data`, not the 1:1 path.
