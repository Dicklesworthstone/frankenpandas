# cod-b Negative-Evidence Ledger

Purpose: record every cod-b optimization attempt in the new performance campaign,
including dead ends, so future agents do not retry failed levers without a concrete
retry predicate.

## 2026-06-18 - br-frankenpandas-uza04.188 - MultiIndex tuple set-op FxHashSet

- Status: implemented, benchmark verdict pending batch-test.
- Lever: replace private `HashMap<Vec<IndexLabel>, ()>` SipHash membership/seen
  maps in `MultiIndex::{intersection, union, difference, symmetric_difference}`
  generic tuple fallback paths with `FxHashSet<Vec<IndexLabel>>`.
- Baseline comparator: current std `HashMap` SipHash tuple-key fallback.
- Graveyard mapping: `alien_cs_graveyard.md` section 7.7 Swiss Tables / high
  performance hash maps, plus the suite summary quick-fix guidance to replace
  default SipHash on non-DoS-facing internal maps.
- Alien-artifact proof obligation: output order remains input-scan driven, not
  hash-iteration driven; tuple membership identity is unchanged; no public
  `HashMap` return type changes.
- Guard added: `multi_index_setop_generic_fallback_preserves_order_codb`,
  forcing the non-packed fallback with 65 levels and checking intersection,
  union, difference, and symmetric difference ordering/dedup semantics.
- Validation run: passed `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo check -p fp-index`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted. Re-ran after the shared checkout advanced to
  `2edf0cf7` with the same pass result.
- Benchmark verdict: pending. Required follow-up comparator is a focused
  MultiIndex set-op workload where mixed-radix packed keys decline and tuple-key
  fallback dominates.
- Retry predicate if rejected: only retry if a profile shows this exact generic
  fallback above 0.1% self-time and a same-host benchmark with this patch reverted
  proves SipHash probe cost, not tuple construction, is the dominant residual.

## 2026-06-18 - br-frankenpandas-uza04.190 - Direct RangeIndex join

- Status: implemented, benchmark verdict pending batch-test.
- Lever: implement `RangeIndex::join` directly for `left`, `right`, `inner`,
  and `outer` instead of forwarding hot typed-Int64 cases through
  `to_flat_index().join(other, how)`.
- Baseline comparator: current flat `Index::join` forwarding path, which builds
  an intermediate flat `Index` wrapper and can materialize the range's Int64
  view before applying membership/union logic.
- Graveyard mapping: cache-aware row-label algebra and specialization: use the
  arithmetic RangeIndex certificate as a compact semantic witness, then stream
  values directly into typed output buffers.
- Alien-artifact proof obligation: `RangeIndex` is unique by construction, so
  inner join only needs membership over the other side, and outer join can emit
  self-order range values followed by first-seen other values not in the range.
  Output ordering, duplicate suppression, name propagation, and invalid-`how`
  errors remain oracle-checked against the old flat `Index::join` path.
- Guard added: `range_index_join_direct_i64_matches_flat_oracle_uza04190`,
  covering left/right/inner/outer, descending ranges, duplicate right labels,
  shared and mismatched names, mixed-label fallback, invalid `how`, and typed
  Int64 output backing for inner/outer.
- Validation run: passed `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo check -p fp-index`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- Benchmark verdict: pending. Required follow-up comparator is a focused
  RangeIndex-vs-typed-Int64 Index join workload against the legacy pandas oracle,
  especially inner/outer joins with duplicate right labels and descending range
  sources.
- Retry predicate if rejected: only retry if the profile shows
  `RangeIndex::join` or `Index::join` flat forwarding above 0.1% self-time and a
  same-host reverted comparison proves intermediate range materialization, not
  downstream `Index` construction, is the residual.
