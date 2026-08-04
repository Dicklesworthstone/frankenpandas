# br-frankenpandas-yq96z dense-cycle one-sided join KEEP

Date: 2026-06-13
Agent: LavenderStone

## Profile-backed target

Bead `br-frankenpandas-yq96z` targets large-output one-sided join
materialization after profiles showed the `left_join` / `right_join`
fanout paths dominated by repeated null-introducing materialization.

Baseline artifact:
`tests/artifacts/perf/yq96z_base_join_fanout.txt`

- `left_join 500000x20`: `707.6 ms +/- 24.7`
- `right_join 500000x20`: `640.1 ms +/- 26.4`
- `outer_join 500000x20`: `75.6 ms +/- 7.9`

## Lever

One lever: add probe-order dense-cycle Int64 lazy lanes for one-sided joins and
route only certified dense-cycle LEFT/RIGHT join shapes to them.

- Kept side: `LazyDenseCycleProbeRepeatInt64`
- Null-introduced build side: `LazyNullableDenseCycleProbeBuildInt64`
- Gate: both key columns must expose exact `Int64DenseCycleWitness` and satisfy
  the existing dense-span bound. Otherwise the old CSR/segment path remains.

This skips the O(probe rows) nullable segment descriptor and per-output-lane
bucket-order value tape for the certified dense-cycle one-sided path.

## Isomorphism proof

Ordering and tie-breaking are unchanged:

- For each probe row `p`, the key is reconstructed as
  `probe.start + (p % probe.period)`, exactly the certified source key.
- If the build side has matches for that key, the source positions emitted are
  `offset, offset + build.period, ...` for `count` rows. That is the same stable
  duplicate order the existing CSR `positions` tape stores for the key.
- If the build side has no match, the build lane emits exactly one
  `Null(NullKind::Null)` and the kept side emits the original probe value once,
  matching the previous `usize::MAX` segment semantics.
- The output index remains the known unique unit range `0..output_len`.
- DTypes are preserved as `Int64`; validity is the same sparse invalid-range
  witness as the previous nullable repeated-slices lane.
- No floating-point arithmetic, RNG, hash iteration order, or parallel write
  ordering is introduced by this route.

Focused tests compare the new route directly with the previous materialized
position-vector builders:

- `cargo test -p fp-columnar dense_cycle_probe --lib`
- `cargo test -p fp-join dense_cycle_probe --lib`

## Golden SHA256

Candidate artifact:
`tests/artifacts/perf/yq96z_dense_cycle_golden_hash_rows.txt`

Comparison:
`tests/artifacts/perf/yq96z_dense_cycle_golden_cmp_status.txt`

`golden_cmp_status=0` against
`tests/artifacts/perf/yq96z_base_golden_hash_rows.txt`.

Rows:

```text
79038819f2e8bd9bb608cac9520688c71dd7e9af2ffb6e61625a974db1388587  left_join 2000
34ca5d52f217ee86b4561e812bca723eb3b6de5db17e16772476cc2b9e4868b3  left_join 20000
889863ce20ca332cadfd1a29eec1ea494a9e55b1469f5f29fe97a8b97bfa9d5c  outer_join 2000
453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750  outer_join 20000
56b205a42ef9398293aa31db9d8bdc3da5ae67aaa7367c861fe52de207ada583  right_join 2000
e1e429e93dd6f69a35a902be8a4954f7136decc2f14daac2212b46bc07792cca  right_join 20000
1200dce94c3e2bbc18a72bdecfa42f4c16a0e6cc347cb048f5d4830278b28f3d  inner_join 2000
c1cf4e8bdc343a24ac5112d6e6290a4ed29fb0f549815779836a7ab74f3c02e9  inner_join 20000
```

## Benchmarks

Candidate build:
`tests/artifacts/perf/yq96z_dense_cycle_build_perf_profile.txt`

Paired hyperfine:
`tests/artifacts/perf/yq96z_dense_cycle_pair_join_fanout.txt`
and `.json`

- `left_join 500000x20`: `729.1 ms +/- 21.5` ->
  `142.2 ms +/- 8.9` (`5.13x` faster)
- `right_join 500000x20`: `673.6 ms +/- 14.1` ->
  `144.0 ms +/- 9.9` (`4.68x` faster)
- `outer_join 500000x20`: `81.0 ms +/- 6.7` ->
  `81.0 ms +/- 8.5` (neutral guardrail; route unchanged)

Score: `Impact 5 x Confidence 5 / Effort 2 = 12.5`, KEEP.

## Validation

- `rch exec -- cargo check -p fp-columnar --lib`: pass on `vmi1153651`
- `rch exec -- cargo check -p fp-join --lib`: pass on `vmi1153651`
- `rch exec -- cargo test -p fp-columnar dense_cycle_probe --lib`: 2 passed
- `rch exec -- cargo test -p fp-join dense_cycle_probe --lib`: 2 passed
- `cargo fmt --check -p fp-columnar -p fp-join`: pass
- `rch exec -- cargo clippy -p fp-columnar -p fp-join --lib -- -D warnings`:
  pass on `vmi1227854`
- `ubs crates/fp-columnar/src/lib.rs crates/fp-join/src/lib.rs`: exit 1 from
  broad file-level inventory/false-positive findings already present in these
  large modules, including dtype/key equality reported as secret comparisons;
  UBS also reports `cargo check`, tests build, formatting, clippy, audit, and
  deny clean.

`perf stat` remains blocked by host policy (`perf_event_paranoid=4`), recorded
in `tests/artifacts/perf/yq96z_base_perf_stat_status.txt`.

## Verdict

KEEP. Close `br-frankenpandas-yq96z` after landing this commit and re-profile
for the next shifted bottleneck.
