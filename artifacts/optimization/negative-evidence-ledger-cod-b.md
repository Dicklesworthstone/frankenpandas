# cod-b Negative-Evidence Ledger

Purpose: record every cod-b optimization attempt in the new performance campaign,
including dead ends, so future agents do not retry failed levers without a concrete
retry predicate.

## 2026-06-18 - br-frankenpandas-uza04.196 - CategoricalIndex monotonic rank scan

- Status: implemented, benchmark verdict pending batch-test.
- Lever: replace `CategoricalIndex::{is_monotonic_increasing,
  is_monotonic_decreasing}` `codes()` materialization with a streaming category
  rank scan over `Option<usize>` ranks.
- Baseline comparator: current monotonic predicates allocate a full
  `Vec<Option<usize>>` through `codes()` and then scan adjacent windows, even
  though the final result only needs the previous and current rank.
- Graveyard mapping: semantic compression and cache-aware streaming scan:
  retain only one prior category-rank witness instead of allocating a full code
  vector for a boolean predicate.
- Alien-artifact proof obligation: `codes()` maps labels to first-occurrence
  category ranks and represents impossible labels as `None`. The streaming
  comparator preserves the exact `Option<usize>` ordering used by
  `codes().windows(2)` while removing the intermediate vector; category order,
  duplicate categories, empty/singleton truth values, and invalid-label
  behavior remain unchanged.
- Guard added:
  `categorical_index_monotonic_scans_ranks_without_codes_vec_uza04196`, covering
  increasing and decreasing categorical rank sequences plus an invalid-label
  case checked directly against the old `codes().windows(..)` comparator.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo check -p fp-index`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- Benchmark verdict: pending. Required follow-up comparator is
  `crates/fp-index/examples/bench_categorical_codes.rs` or a focused categorical
  monotonic criterion lane on repeated realistic categorical indexes versus the
  legacy pandas original and a pre-patch `codes()` allocation baseline.
- Retry predicate if rejected: only retry if same-host profiling shows
  categorical monotonic predicates above 0.1% self-time and allocation profiling
  proves `codes()` vector construction, not category rank hash lookups, is the
  residual.

## 2026-06-18 - br-frankenpandas-uza04.195 - MultiIndex nunique direct count

- Status: implemented, benchmark verdict pending batch-test.
- Lever: replace `MultiIndex::nunique()` `unique().len()` materialization with
  direct unique tuple counting, using packed `u64` identity keys when available
  and `FxHashSet<Vec<IndexLabel>>` fallback when the mixed-radix key space
  overflows.
- Baseline comparator: current path builds the full unique `MultiIndex`
  through duplicate-mask computation and position gathers just to observe its
  length.
- Graveyard mapping: semantic compression and count-only specialization:
  retain only tuple identity keys required for cardinality, not the unique
  output structure.
- Alien-artifact proof obligation: `nunique` observes only tuple cardinality,
  so first-seen unique ordering, names, and output-level construction are
  unobservable. Missing labels remain part of the tuple identity through the same
  `IndexLabel` equality/hash semantics used by `unique`.
- Guard added: `multi_index_nunique_counts_without_unique_output_uza04195`,
  covering the packed-key path and a 65-level fallback case that forces
  mixed-radix overflow, both checked against `unique().len()`.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo check -p fp-index`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- Benchmark verdict: pending. Required follow-up comparator is criterion
  `MultiIndex::nunique` on repeated mixed Utf8/Int64 tuple keys versus the
  legacy pandas original and a pre-patch unique-output baseline.
- Retry predicate if rejected: only retry if same-host profiling shows
  `MultiIndex::nunique` above 0.1% self-time and direct key counting is not
  dominated by unavoidable tuple/key construction.

## 2026-06-18 - br-frankenpandas-uza04.194 - MultiIndex tuple maps FxHashMap

- Status: implemented, benchmark verdict pending batch-test.
- Lever: replace `MultiIndex::{factorize, value_counts}` tuple-key
  `std::collections::HashMap` maps with capacity-sized `FxHashMap` maps.
- Baseline comparator: current SipHash maps over `Vec<IndexLabel>` tuple keys
  in repeated realistic MultiIndex factorization and tuple counting.
- Graveyard mapping: Swiss Tables / fast non-cryptographic internal maps:
  tuple keys are internal, non-adversarial benchmark/conformance data, and both
  public outputs are order-defined outside hash-table iteration.
- Alien-artifact proof obligation: factorize codes and uniques still follow
  first-seen input row order; value_counts still sorts by count descending and
  then tuple ordering, so map iteration order cannot leak into observable
  output. MultiIndex names propagate unchanged.
- Guard added: `multi_index_factorize_value_counts_fxhash_order_uza04194`,
  covering duplicate tuple factorization, first-seen uniques, name propagation,
  and count-tie sorting independent of hash iteration.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo check -p fp-index`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- Benchmark verdict: pending. Required follow-up comparator is criterion
  `MultiIndex::factorize` and `MultiIndex::value_counts` on repeated mixed
  Utf8/Int64 tuple keys versus the legacy pandas original and a pre-patch
  SipHash baseline.
- Retry predicate if rejected: only retry if same-host profiling shows these
  exact tuple map paths above 0.1% self-time and tuple cloning, not hashing, is
  proven not to dominate the residual.

## 2026-06-18 - br-frankenpandas-uza04.155 - RangeIndex drop direct mask

- Status: implemented, benchmark verdict pending batch-test.
- Lever: replace `RangeIndex::drop(labels)` `FxHashSet<i64>` construction and
  per-range-value hash probes with direct drop-position marking through
  `RangeIndex::position_of_value`.
- Baseline comparator: current RangeIndex drop path, which hashes every Int64
  drop label, ignores non-Int64 labels, then probes the set once per range
  position before building the typed Int64 result.
- Graveyard mapping: bitset/bitmap marking plus algebraic RangeIndex
  specialization: use the compact `(start, step, len)` witness to map a label
  directly to an output position instead of paying hash-table allocation and
  probe cache misses.
- Alien-artifact proof obligation: duplicate labels collapse to the same
  dropped position; missing labels leave the mask unchanged; non-Int64 labels
  remain ignored; output order, name propagation, empty-range behavior, and typed
  Int64 backing stay identical to the previous implementation.
- Guard added: `range_index_drop_marks_positions_without_hash_uza04155`,
  covering descending ranges, duplicate drop labels, non-Int64 labels, misses,
  empty ranges, name preservation, and typed Int64 output backing.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo check -p fp-index`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- Benchmark verdict: pending. Required follow-up comparator is criterion
  `RangeIndex::drop` on 1M-row ascending/descending ranges with small and large
  label-drop lists versus the legacy pandas original and a pre-patch hash-probe
  baseline.
- Retry predicate if rejected: only revisit if a same-host benchmark shows
  `RangeIndex::drop` above 0.1% self-time and allocation profiling proves the
  residual is position-mask allocation rather than typed `Index` construction.

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
