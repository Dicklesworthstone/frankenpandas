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

## 2026-06-18 - br-frankenpandas-iatnc - Closed-form RangeIndex set ops

- Status: implemented, benchmark verdict pending batch-test.
- Lever: replace `RangeIndex::{intersection, union, difference,
  symmetric_difference}` `FxHashSet<i64>` membership/seen maps with direct
  arithmetic membership checks through `RangeIndex::contains_value`.
- Baseline comparator: current RangeIndex-vs-RangeIndex set-op path, which builds
  one to three hash sets even though both operands are unique arithmetic
  progressions.
- Graveyard mapping: algebraic data representation and cache-oblivious scanning:
  use `(start, step, len)` as a semantic witness and stream each side exactly
  once with no hash-table allocation or probe cache misses.
- Alien-artifact proof obligation: `RangeIndex` is unique by construction, so
  seen-set deduplication is redundant. Membership in the opposite range is
  closed-form; output order remains self-order for intersection/difference and
  self-then-other order for union/symmetric difference. Name propagation and
  typed Int64 backing are unchanged.
- Guard added:
  `range_index_set_ops_closed_form_membership_preserves_order_iatnc`, covering
  overlapping descending ranges, disjoint ranges, pandas-order outputs, name
  propagation, and typed Int64 output backing.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo check -p fp-index`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- Benchmark verdict: pending. Required follow-up comparator is criterion
  `RangeIndex` set ops on overlapping/disjoint 1M-row ascending and descending
  ranges versus the legacy pandas original and a pre-patch hash-set baseline.
- Retry predicate if rejected: only revisit if a same-host benchmark proves
  `RangeIndex::{intersection, union, difference, symmetric_difference}` is still
  above 0.1% self-time and the residual is arithmetic membership rather than
  typed `Index` construction or output allocation.

## 2026-06-18 - br-frankenpandas-29u49 - RangeIndex miss-heavy indexers

- Status: implemented, benchmark verdict pending batch-test.
- Lever: add internal `RangeIndex::position_of_value` and route
  `RangeIndex::{get_indexer, get_indexer_non_unique, reindex}` through it instead
  of calling public `get_loc` for every target.
- Baseline comparator: vectorized RangeIndex indexer paths that allocate an
  `IndexError::InvalidArgument` string for each normal miss before mapping it to
  `-1`.
- Graveyard mapping: devirtualized sentinel-return lookup and negative-result
  fast path: keep exceptions at API boundaries and use branch-predictable
  `Option<usize>` inside bulk kernels.
- Alien-artifact proof obligation: public `get_loc` error behavior remains
  unchanged, including the invalid zero-step guard. Bulk indexers still produce
  the same hit positions, `-1` miss markers, missing target positions, and
  reindexed target identity.
- Guard added: `range_index_vectorized_indexers_match_get_loc_29u49`, covering
  descending RangeIndex hits/misses, `get_indexer`, `get_indexer_non_unique`,
  `reindex`, and the public missing `get_loc` error string.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo check -p fp-index`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- Benchmark verdict: pending. Required follow-up comparator is criterion
  miss-heavy `RangeIndex::get_indexer`/`reindex` on 1M targets versus the legacy
  pandas original and a pre-patch `get_loc` error-allocation baseline.
- Retry predicate if rejected: only revisit if a same-host benchmark shows these
  vectorized RangeIndex indexers above 0.1% self-time and allocation profiling
  confirms miss-error construction is still material.

## 2026-06-18 - br-frankenpandas-ruthb - RangeIndex isin direct mask

- Status: implemented, benchmark verdict pending batch-test.
- Lever: replace `RangeIndex::isin` needle `FxHashSet<i64>` construction plus
  per-row hash probes with direct output-mask marking via
  `RangeIndex::position_of_value`.
- Baseline comparator: current `RangeIndex::isin(values)` path, which hashes all
  needles and probes that table once per range row.
- Graveyard mapping: bitset/bitmap marking and cache-aware linear writes: map
  semantic keys directly to output positions and avoid hash probes entirely.
- Alien-artifact proof obligation: `isin` returns a mask in index-position order;
  duplicate needles collapse to the same boolean slot; missing needles leave the
  mask unchanged; empty ranges return an empty mask.
- Guard added: `range_index_isin_marks_positions_without_hash_ruthb`, covering
  ascending and descending ranges, duplicate needles, misses, and empty ranges.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo check -p fp-index`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- Benchmark verdict: pending. Required follow-up comparator is criterion
  `RangeIndex::isin` on 1M-row ascending/descending ranges with small and large
  needle arrays versus the legacy pandas original and a pre-patch hash-probe
  baseline.
- Retry predicate if rejected: only retry if a same-host benchmark shows
  `RangeIndex::isin` above 0.1% self-time and the profile attributes cost to
  hash-table probing rather than output-mask allocation.
