# cod-b Negative-Evidence Ledger

Purpose: record every cod-b optimization attempt in the new performance campaign,
including dead ends, so future agents do not retry failed levers without a concrete
retry predicate.

## 2026-06-18/19 - Gauntlet verification - br-frankenpandas-uza04.205/.206 affine take cluster

- Harness: `benches/vs_pandas_harness.py --category indexing --workloads
  range_index_take_arithmetic,affine_index_take_arithmetic --sizes 100k,1M
  --dtypes float64`, `TAKE_BATCH=256`, pinned with `taskset` for accepted rows.
- Subject: FrankenPandas `fp-bench` built with
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo build
  --profile release-perf -p fp-bench`.
- Oracle: pandas 2.2.3.
- Criterion guard: `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b
  rch exec -- cargo bench -p fp-conformance --bench vs_pandas --
  take_arithmetic`.
- Final action: keep br-frankenpandas-uza04.205 `RangeIndex::take` arithmetic
  selector laziness as a measured FP-side improvement, but record it as still
  slower than pandas; revert br-frankenpandas-uza04.206 generic affine
  `Index::take` arithmetic laziness because it regressed the generic affine
  workload versus the pre-optimization FP baseline and pandas.

### Accepted final post-revert rows

| Workload | Rows | FP p50 | pandas p50 | Ratio vs pandas | Verdict | Artifact |
|---|---:|---:|---:|---:|---|---|
| `range_index_take_arithmetic` | 1M | 83.685 ms | 62.712 ms | 0.749x | SLOWER | `artifacts/bench/gauntlet_cod_b_range_take_after_revert206_vs_pandas_batch256_taskset7.json` |
| `affine_index_take_arithmetic` | 100k | 7.200 ms | 6.001 ms | 0.833x | SLOWER | `artifacts/bench/gauntlet_cod_b_range_take_after_revert206_vs_pandas_batch256_taskset7.json` |
| `affine_index_take_arithmetic` | 1M | 72.051 ms | 54.687 ms | 0.759x | SLOWER | `artifacts/bench/gauntlet_cod_b_range_take_after_revert206_vs_pandas_batch256_taskset7.json` |

### Accepted pre-optimization comparator

| Workload | Rows | Preopt FP p50 | pandas p50 | Ratio vs pandas | Post-revert FP p50 | FP delta | Decision |
|---|---:|---:|---:|---:|---:|---:|---|
| `range_index_take_arithmetic` | 100k | 12.187 ms | 7.643 ms | 0.627x | no accepted row; noisy p50 8.16-8.49 ms | routing only | keep based on 1M accepted row |
| `range_index_take_arithmetic` | 1M | 127.438 ms | 62.487 ms | 0.490x | 83.685 ms | 1.52x faster FP-side | KEEP `.205`, still slower than pandas |
| `affine_index_take_arithmetic` | 100k | 6.965 ms | 5.280 ms | 0.758x | 7.200 ms | 0.97x FP-side | `.206` reverted |
| `affine_index_take_arithmetic` | 1M | 72.892 ms | 52.949 ms | 0.726x | 72.051 ms | 1.01x FP-side | `.206` reverted; no material gain |

### Rejected intermediate `.206` rows before revert

| Workload | Rows | FP p50 | pandas p50 | Ratio vs pandas | Verdict | Artifact |
|---|---:|---:|---:|---:|---|---|
| `range_index_take_arithmetic` | 100k | 10.708 ms | 7.647 ms | 0.714x | SLOWER | `artifacts/bench/gauntlet_cod_b_range_take_vs_pandas_batch256_taskset7.json` |
| `range_index_take_arithmetic` | 1M | 107.195 ms | 58.252 ms | 0.543x | SLOWER | `artifacts/bench/gauntlet_cod_b_range_take_1m_batch256_taskset7_rerun.json` |
| `affine_index_take_arithmetic` | 100k | 11.350 ms | 5.278 ms | 0.465x | SLOWER | `artifacts/bench/gauntlet_cod_b_range_take_vs_pandas_batch256_taskset7.json` |
| `affine_index_take_arithmetic` | 1M | 114.805 ms | 52.543 ms | 0.458x | SLOWER | `artifacts/bench/gauntlet_cod_b_range_take_vs_pandas_batch256_taskset7.json` |

### Dropped/noise rows kept for non-retry context

| Artifact | Result |
|---|---|
| `artifacts/bench/gauntlet_cod_b_range_take_vs_pandas.json` | `TAKE_BATCH=16`; all four rows dropped by CV. Provisional medians already trailed pandas, so batch was increased instead of accepting noisy data. |
| `artifacts/bench/gauntlet_cod_b_range_take_vs_pandas_batch64.json` | `affine_index_take_arithmetic` accepted and slower at 100k/1M (0.582x, 0.477x); both RangeIndex rows dropped. |
| `artifacts/bench/gauntlet_cod_b_range_take_vs_pandas_batch256.json` | only RangeIndex 1M accepted and slower (0.544x); other rows dropped. |
| `artifacts/bench/gauntlet_cod_b_range_take_after_revert206_vs_pandas_batch256_taskset7.json` | final RangeIndex 100k row dropped: FP p50 8.227 ms, pandas p50 8.571 ms, FP CV 14.83%. |
| `artifacts/bench/gauntlet_cod_b_range_take_100k_after_revert206_batch256_taskset7_rerun.json` | final RangeIndex 100k rerun dropped: FP p50 8.160 ms, pandas p50 8.441 ms, FP CV 7.02%. |
| `artifacts/bench/gauntlet_cod_b_range_take_100k_after_revert206_batch256_taskset8_rerun.json` | final RangeIndex 100k rerun dropped: FP p50 8.485 ms, pandas p50 7.659 ms, FP CV 5.82%. |

### Retry rules from this gauntlet

- Do not retry the generic affine `Index::take` arithmetic-selector lazy-output
  lever as implemented in `.206`. Its selector scan plus lazy constructor did
  not beat the existing `take_i64_values` path and was materially slower than
  pandas. A retry must use a different primitive: caller-supplied arithmetic
  selector metadata, a range/slice selector type that avoids rescanning the
  selector vector, or a downstream DataFrame/Series path that never constructs
  the position vector.
- RangeIndex `.205` is allowed to remain, but it is not a domination result.
  The next bead should attack the residual O(k) arithmetic-selector scan or
  introduce a first-class `RangeTake`/slice selector witness so both FP and
  pandas avoid dense position-vector work.

## 2026-06-18 - br-frankenpandas-uza04.206 - Affine Index take selectors

- Status: reverted after gauntlet; benchmark verdict rejected.
- Lever: detect arithmetic selectors in generic `Index::take` when the source
  index already has lazy affine Int64 backing, and return a lazy affine Int64
  `Index` instead of allocating the typed output vector.
- Baseline comparator: the pre-patch `Index::take` used `take_i64_values` for
  lazy affine labels, which avoided enum-label materialization but still wrote
  every selected scalar into a fresh `Vec<i64>`.
- Graveyard mapping: selection-vector specialization and semantic witnesses:
  the source affine label certificate plus an arithmetic selector certificate
  compose into a new affine label certificate, so output labels can stay symbolic
  until a caller asks for concrete labels.
- Alien-artifact proof obligation: the fast path only fires for existing affine
  Int64 backing; out-of-bounds selectors return to the old panic path;
  duplicate/non-arithmetic selectors fall back to typed vectors; descending
  selectors keep negative affine steps; checked stride multiplication and
  affine construction reject unrepresentable strides without changing output.
- Guard removed with the reverted fast path:
  `affine_int64_take_arithmetic_selectors_keep_lazy_uza04206`,
  checking ascending, descending, singleton, empty, duplicate fallback,
  irregular fallback, name propagation, lazy backing, and label equality through
  the public typed view.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo check -p fp-index`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- Benchmark verdict: rejected by the 2026-06-18/19 gauntlet above. The
  intermediate fast path measured 0.465x pandas at 100k and 0.458x pandas at
  1M, while the pre-optimization path measured 0.758x and 0.726x respectively.
  The code was reverted in the gauntlet commit.
- Retry predicate if rejected: only retry if same-worker profiling shows
  affine `Index::take` or downstream reorder index gathering above 0.1%
  self-time and allocation profiling proves typed output-vector construction is
  still the residual after excluding non-affine selectors.

## 2026-06-18 - br-frankenpandas-uza04.205 - RangeIndex take affine selectors

- Status: kept after gauntlet as a measured FP-side improvement, still slower
  than pandas.
- Lever: detect arithmetic `RangeIndex::take` position selectors and return a
  lazy affine Int64 `Index` instead of materializing every output label into a
  `Vec<i64>`.
- Baseline comparator: the pre-patch path bounds-checked positions, computed
  every selected label, allocated a typed output vector, and deferred only the
  enum-label materialization. Common iloc/list-take shapes such as contiguous,
  stepped, reversed, singleton, and empty selectors carry enough affine witness
  data to skip that output vector entirely.
- Graveyard mapping: selection-vector and witness-carry specialization: keep
  the arithmetic selector as a semantic certificate and propagate it into an
  affine backing, falling back to typed vectors for duplicate or irregular
  gathers where uniqueness/stride witnesses are invalid.
- Alien-artifact proof obligation: bounds errors are still raised before the
  fast path; duplicate selectors cannot use the known-unique affine constructor;
  descending selectors keep negative affine steps; empty/singleton selectors
  preserve labels and name; checked step multiplication and affine construction
  fall back to the old typed output if the stride cannot be represented safely.
- Guard added: `range_index_take_arithmetic_keep_affine_uza04205`, checking
  ascending, descending, singleton, empty, duplicate fallback, name propagation,
  materialization avoidance, and label equality through the public typed view.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo check -p fp-index`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- Benchmark verdict: accepted as partial FP-side win by the 2026-06-18/19
  gauntlet above. The accepted 1M row improved FP p50 from 127.438 ms preopt to
  83.685 ms post-revert, but still trailed pandas at 0.749x. No domination
  claim is made.
- Retry predicate if rejected: only retry if same-worker profiling shows
  `RangeIndex::take` or index gather materialization above 0.1% self-time and
  allocation profiling proves the residual is typed output-vector construction,
  not downstream label materialization or non-affine selector handling.

## 2026-06-18 - br-frankenpandas-uza04.201 - CategoricalIndex factorize rank codes

- Status: implemented, benchmark verdict pending batch-test.
- Lever: replace `CategoricalIndex::factorize()` full-label hash code assignment
  with rank-indexed code vectors for bounded valid category domains plus
  invalid-label fallback codes.
- Baseline comparator: current `factorize` hashes every label string into a
  first-seen position map, even when valid categorical labels can be mapped to
  compact dictionary ranks and assigned codes through a dense vector.
- Graveyard mapping: semantic compression and dictionary-coded execution:
  factorize over category ranks rather than repeated string identities, while
  keeping string fallback only for impossible deserialized labels or sparse
  oversized category universes.
- Alien-artifact proof obligation: codes still encode first-seen label order,
  not category order; unique labels are pushed when each label first appears;
  invalid labels share codes by string equality; categories, ordered flag, and
  name propagate unchanged. Oversized category domains fall back to direct label
  hashing to avoid O(k) metadata work.
- Guard added: `categorical_index_factorize_uses_rank_codes_uza04201`, checking
  first-seen codes, unique-label output, metadata propagation, invalid-label
  fallback, and oversized-category fallback against a local hash-oracle.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo check -p fp-index`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- Benchmark verdict: pending. Required follow-up comparator is focused
  criterion for `CategoricalIndex::factorize` on repeated low-cardinality
  realistic categorical indexes versus the legacy pandas original and a
  pre-patch full-label hash-code baseline.
- Retry predicate if rejected: only retry if same-host profiling shows
  categorical factorization above 0.1% self-time and allocation profiling proves
  per-label hash code assignment, not unique-label output cloning, is the
  residual.

## 2026-06-18 - br-frankenpandas-uza04.200 - CategoricalIndex value_counts rank counts

- Status: implemented, benchmark verdict pending batch-test.
- Lever: replace `CategoricalIndex::value_counts()` per-label hash counting
  with rank-indexed count vectors for bounded category domains plus invalid-label
  fallback counts.
- Baseline comparator: current `value_counts` hashes every label string into a
  full-label map while preserving first-seen order in a side vector, even when
  valid categorical labels can be counted by dictionary rank.
- Graveyard mapping: counting-sort style semantic compression: use the
  categorical dictionary as a dense identity space, store counts in cache-local
  `Vec<usize>`, and retain hash counting only for impossible deserialized labels
  or oversized sparse category universes.
- Alien-artifact proof obligation: pre-sort pair order remains first-seen label
  order, so stable descending-count sorting preserves pandas-observable tie
  order. Valid labels count by first category rank; invalid labels count by
  string value; unused categories never appear; oversized metadata falls back to
  direct label hashing.
- Guard added: `categorical_index_value_counts_use_rank_counts_uza04200`,
  checking descending count order, first-seen tie order, invalid-label fallback,
  unused categories, and oversized-category fallback against a local first-seen
  count oracle.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo check -p fp-index`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- Benchmark verdict: pending. Required follow-up comparator is focused
  criterion for `CategoricalIndex::value_counts` on repeated low-cardinality
  realistic categorical indexes versus the legacy pandas original and a
  pre-patch full-label hash-counting baseline.
- Retry predicate if rejected: only retry if same-host profiling shows
  categorical value counts above 0.1% self-time and allocation profiling proves
  per-label hash counting, not final pair construction or stable sort, is the
  residual.

## 2026-06-18 - br-frankenpandas-uza04.199 - CategoricalIndex unique/drop_duplicates rank bitset

- Status: implemented, benchmark verdict pending batch-test.
- Lever: replace `CategoricalIndex::{unique,drop_duplicates}` full-label
  `FxHashSet` construction with first-seen output construction over
  category-rank bitsets plus invalid-label fallback sets.
- Baseline comparator: current `unique` hashes every full label string to decide
  first-seen output membership, and `drop_duplicates` forwards through that same
  hash path even when the categorical dictionary already supplies compact valid
  label identity.
- Graveyard mapping: bitmap membership and witness-carry specialization:
  reuse the categorical dictionary as a rank witness, turn valid-label
  membership into one bit per category, and retain string hashing only for
  impossible deserialized labels or oversized unused category universes.
- Alien-artifact proof obligation: first-seen order is still driven by the input
  label scan; valid labels push exactly once per first category rank; invalid
  labels push exactly once per first string value; categories, ordered flag, and
  name are copied unchanged. Oversized category domains keep the old direct
  label-hash fallback so sparse metadata cannot dominate the lane.
- Guard added:
  `categorical_index_unique_drop_duplicates_use_rank_bitset_uza04199`, checking
  first-seen output order, category/name/ordered propagation, invalid-label
  fallback, `drop_duplicates()==unique`, and oversized-category fallback against
  a local first-seen label oracle.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo check -p fp-index`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- Benchmark verdict: pending. Required follow-up comparator is focused
  criterion for `CategoricalIndex::{unique,drop_duplicates}` on repeated
  low-cardinality realistic categorical indexes versus the legacy pandas
  original and a pre-patch full-label hash-set baseline.
- Retry predicate if rejected: only retry if same-host profiling shows
  categorical label-producing uniqueness above 0.1% self-time and allocation
  profiling proves label hashing, not category-map construction or output
  cloning, is the residual.

## 2026-06-18 - br-frankenpandas-uza04.198 - CategoricalIndex duplicated rank bitset

- Status: implemented, benchmark verdict pending batch-test.
- Lever: replace `CategoricalIndex::duplicated(keep)` flat `Index`
  materialization with direct duplicate-mask construction over category-rank
  bitsets plus invalid-label fallback sets.
- Baseline comparator: current path clones every categorical label into
  `IndexLabel::Utf8`, builds a flat `Index`, then runs the generic duplicate
  mask algorithm even though category ranks already provide compact valid-label
  identity.
- Graveyard mapping: bitmap membership and witness-carry specialization:
  reuse the categorical dictionary as a semantic witness and compute duplicate
  masks directly without constructing enum row labels.
- Alien-artifact proof obligation: for valid labels, first-occurrence
  `category_index_map` ranks preserve label equality; for impossible
  deserialized labels, fallback string sets preserve old flat-index equality.
  `keep=first`, `keep=last`, and `keep=false` all match the old flat
  `Index::duplicated` masks, while oversized unused category universes fall back
  to direct label hashing instead of O(k) bitset work.
- Guard added:
  `categorical_index_duplicated_uses_rank_bitset_uza04198`, checking all
  duplicate keep modes against `to_flat_index().duplicated(...)` for repeated
  valid labels, invalid labels, and oversized-category fallback.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo check -p fp-index`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- Benchmark verdict: pending. Required follow-up comparator is focused
  criterion for `CategoricalIndex::duplicated` on low-cardinality realistic
  categorical indexes across keep modes versus the legacy pandas original and
  pre-patch flat-index materialization baseline.
- Retry predicate if rejected: only retry if same-host profiling shows
  categorical duplicate masks above 0.1% self-time and allocation profiling
  proves flat-index materialization, not category-map construction, is the
  residual.

## 2026-06-18 - br-frankenpandas-uza04.197 - CategoricalIndex unique rank bitset

- Status: implemented, benchmark verdict pending batch-test.
- Lever: replace `CategoricalIndex::{is_unique,nunique}` full-label
  `FxHashSet<&String>` construction with a bounded category-rank bitset scan and
  invalid-label fallback set.
- Baseline comparator: current uniqueness/cardinality paths hash every label
  into a set even when realistic categorical indexes already carry a compact
  category dictionary whose ranks are enough to identify valid labels.
- Graveyard mapping: bitmap membership plus semantic compression: convert label
  equality into category-rank bit tests for valid categorical domains, retaining
  only one bit per category and short-circuiting `is_unique` on the first repeat.
- Alien-artifact proof obligation: valid labels are first-occurrence category
  ranks produced by the existing `category_index_map`; duplicate categories keep
  first-rank semantics. Deserialized impossible labels are not dropped: they are
  counted/probed in a fallback `FxHashSet<&str>`, preserving the old label-string
  equality behavior. Oversized unused category universes fall back to the old
  label-hash scan to avoid replacing an O(n) label path with O(k) metadata work.
- Guard added:
  `categorical_index_unique_nunique_use_rank_bitset_uza04197`, covering repeated
  valid labels, unused categories, unique valid labels, invalid-label fallback,
  and the oversized-category fallback path.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo check -p fp-index`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- Benchmark verdict: pending. Required follow-up comparator is a focused
  `CategoricalIndex::{is_unique,nunique}` criterion lane over repeated
  low-cardinality categorical indexes versus the legacy pandas original and a
  pre-patch full-label hash-set baseline.
- Retry predicate if rejected: only retry if same-host profiling shows
  categorical uniqueness/cardinality above 0.1% self-time and the residual is
  proven to be label hashing rather than category-map construction.

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

## 2026-06-18/20 - br-frankenpandas-iatnc - Closed-form + affine RangeIndex set ops

- Status: measured keep for single-span outputs; residual split-span loss routed.
- Lever 1: replace `RangeIndex::{intersection, union, difference,
  symmetric_difference}` `FxHashSet<i64>` membership/seen maps with direct
  arithmetic membership checks through `RangeIndex::contains_value`.
- Lever 2: when same-step/same-lattice set ops produce a single arithmetic
  progression, return lazy affine Int64 labels instead of materializing a typed
  `Vec<i64>`.
- Baseline comparator: pre-affine `origin/main` RangeIndex-vs-RangeIndex set-op
  path on worker `hz2`, using the same example harness copied into a detached
  baseline worktree at `f83cf68c`.
- Graveyard mapping: run-length/interval containers, affine loop nests, and
  late materialization. Treat `(start, step, len)` as the semantic witness and
  carry the arithmetic progression across the boundary instead of allocating the
  labels.
- Alien-artifact proof obligation: `RangeIndex` is unique by construction, and
  same-lattice overlap/difference over a signed affine progression can be proven
  in position space. Output order, name propagation, empty outputs, descending
  steps, and pandas-style self-order semantics are preserved; split-span
  symmetric differences intentionally fall back.
- Guards added:
  `range_index_set_ops_closed_form_membership_preserves_order_iatnc` and
  `range_index_set_ops_return_affine_spans_iatnc`, covering overlapping
  descending ranges, disjoint/adjacent ranges, pandas-order outputs, name
  propagation, typed fallback backing, and lazy affine output backing.
- Validation runs:
  - `RCH_WORKER=hz2 CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo test -p fp-index range_index_set_ops_return_affine_spans_iatnc -- --nocapture`
    passed on 2026-06-20.
  - `RCH_WORKER=hz2 CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo test -p fp-index range_index_set_ops -- --nocapture`
    passed on 2026-06-20 with all five focused set-op tests green.
  - `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo check -p fp-index --all-targets`
    passed on worker `vmi1293453`.
  - `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo clippy -p fp-index --all-targets -- -D warnings`
    passed locally after `hz2` and `vmi1149989` reported missing remote clippy
    components for the pinned nightly.
  - `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo build --release -p fp-index --example bench_range_setops`
    fell back local due worker-slot pressure and passed.
  - `rustfmt --check crates/fp-index/examples/bench_range_setops.rs` passed.
  - `timeout 180s ubs crates/fp-index/src/lib.rs crates/fp-index/examples/bench_range_setops.rs`
    exited 0 with no critical findings; warnings were the known broad
    `fp-index/src/lib.rs` inventory.

### 2026-06-20 same-worker pre/post evidence

| Operation | `origin/main` hz2 | affine-spans hz2 | FP delta | Verdict |
|---|---:|---:|---:|---|
| `intersection` | 9.240731 ms | 0.000100 ms | 92,407x faster | KEEP |
| `union` | 10.632178 ms | 0.000090 ms | 118,135x faster | KEEP |
| `difference` | 9.341052 ms | 0.000100 ms | 93,411x faster | KEEP |
| `symmetric_difference` | 18.670185 ms | 18.843325 ms | 0.991x | NEUTRAL / route split-span |

Command:
`RCH_WORKER=hz2 CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo run -p fp-index --example bench_range_setops --release -- 1000000 200 overlap`.

### 2026-06-20 pandas head-to-head

| Operation | FrankenPandas | pandas 2.2.3 | Ratio vs pandas | Verdict |
|---|---:|---:|---:|---|
| `intersection` | 120 ns | 9,018 ns | 75.15x faster | WIN |
| `union` | 120 ns | 7,995 ns | 66.63x faster | WIN |
| `difference` | 130 ns | 16,742 ns | 128.78x faster | WIN |
| `symmetric_difference` | 16.868715 ms | 8.619318 ms | 0.51x | LOSS |

Win/loss/neutral ratio vs pandas: **3 / 1 / 0**.

Negative evidence: overlapping `symmetric_difference` produces two disjoint
spans, and the current `Index` backing can represent only one affine run. The
membership lever is not the limiting primitive anymore. Do not retry more hash
or scalar-membership micro-tuning here; create a first-class lazy multi-span
Int64/run container or route `symmetric_difference` through a two-run backing.

Retry predicate: only reopen `br-frankenpandas-iatnc` if a same-worker benchmark
shows the single-span affine outputs regressed or a conformance witness proves
name/order/descending-step semantics changed. Otherwise target the new
multi-span representation bead for the recorded pandas loss.

## 2026-06-20 - br-frankenpandas-uza04.168 - RangeIndex split symmetric_difference two-run backing

- Status: measured KEEP; closes the residual split-span loss from
  `br-frankenpandas-iatnc`.
- Lever: represent an overlapping `RangeIndex::symmetric_difference` result as
  two boxed affine Int64 runs when the left-only and right-only segments are
  disjoint. This keeps construction/`len()` O(1), preserves materialization only
  at the `labels()`/typed-view boundary, and avoids inflating every `Index`
  instance by boxing the rare two-run descriptor.
- Graveyard/artifact mapping: lazy multi-run region layout plus witness-ledger
  preservation. The witness is explicit: the two runs are constructed only from
  validated `RangeIndex` endpoints and steps; adjacent runs still collapse into
  the existing single affine span; materialized and typed consumers generate the
  exact same ordered Int64 labels.

### 2026-06-20 measured evidence

| Comparator | Workload | Time | Notes |
|---|---|---:|---|
| pandas 2.2.3 local p50 | 1M overlap, `len(left.symmetric_difference(right))` | 5.157781 ms | same-host head-to-head, best 5.050482 ms |
| FrankenPandas local release | exact boxed two-run code, same workload | 0.000110 ms | `bench_range_setops`; ratio vs pandas **46,889x faster** |
| FrankenPandas rch release | exact boxed two-run code, worker `vmi1227854` | 0.000140 ms | `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo run -p fp-index --example bench_range_setops --release -- 1000000 200 overlap` |
| FrankenPandas pre-change remote baseline | fresh-restart baseline, worker `vmi1293453` | 39.710381 ms | same command before the two-run backing; routing evidence for FP-side win |

Win/loss/neutral ratio vs pandas for this pass: **1 / 0 / 0**.

### Guards

- `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo check -p fp-index --all-targets` passed on worker `vmi1227854`.
- `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo clippy -p fp-index --all-targets -- -D warnings` passed locally. Remote clippy was attempted first and failed because worker `vmi1264463` lacked `cargo-clippy` for `nightly-2026-04-22`.
- `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo test -p fp-index range_index_set_ops_return_affine_spans_iatnc -- --nocapture` passed after boxing the two-run descriptor.
- `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo run -p fp-index --example golden_isin_symdiff_i64 --release` passed (`ALL GOLDEN CHECKS PASSED`).
- `cargo fmt -p fp-index --check` still reports broad pre-existing formatting
  drift in `crates/fp-index/src/lib.rs`; a targeted scan of the new two-run
  hunk returned no matches after manual rustfmt-shape fixes.
- `timeout 180s ubs crates/fp-index/src/lib.rs` exited 0. UBS reported the broad
  existing `fp-index` warning inventory, no critical issues, and clean internal
  fmt/clippy/check/test-build gates.

Retry predicate: do not retry hash/set-membership micro-levers for overlapping
`RangeIndex::symmetric_difference`. Reopen only if a forced materialization
consumer becomes the next measured loss, or if a future multi-span generalization
can subsume this two-run special case without regressing construction latency.

## 2026-06-18/19 - br-frankenpandas-29u49 - RangeIndex miss-heavy indexers

- Status: measured; keep as FP-side improvement, not pandas-ready.
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
- Benchmark verdict: accepted mixed evidence. Focused Criterion measured the
  current bulk path against a bench-local legacy model that calls public
  `get_loc` for every target and maps misses to `-1`. pandas 2.2.3 public API
  timings used prebuilt target objects so construction stayed outside the timed
  window, matching the Criterion setup.

| Workload | Rows | Current FP median | Legacy model median | pandas median | Ratio vs pandas | FP speedup vs legacy | Verdict |
|---|---:|---:|---:|---:|---:|---:|---|
| `get_indexer`, 15/16 misses | 100k | 1.344 ms | 5.139 ms | 1.110 ms | 0.825x | 3.82x | SLOWER_THAN_PANDAS |
| `get_indexer`, 15/16 misses | 1M | 10.744 ms | 49.907 ms | 16.435 ms | 1.530x | 4.65x | DROPPED_HIGH_CV (pandas CV 5.40%) |
| `reindex`, all misses | 100k | 1.150 ms | 5.341 ms | 0.990 ms | 0.860x | 4.64x | SLOWER_THAN_PANDAS |
| `reindex`, all misses | 1M | 12.285 ms | 50.447 ms | 13.127 ms | 1.069x | 4.11x | NEUTRAL_VS_PANDAS |

- Decision: no revert. This is not a ~0-gain optimization: the retained path is
  3.82x-4.65x faster than the legacy error-allocation model. It is also not a
  pandas-domination result; pandas still wins the accepted 100k rows and the 1M
  reindex row is inside the neutral band. The next useful lever is output/vector
  allocation or pandas-style vectorized engine behavior, not returning to
  exception-driven misses.
- Artifacts:
  `artifacts/bench/gauntlet_cod_b_range_indexers_vs_pandas.json`,
  `artifacts/bench/gauntlet_cod_b_range_indexers_criterion_local.txt`,
  `artifacts/bench/gauntlet_cod_b_range_indexers_criterion_rch.txt`,
  `artifacts/bench/gauntlet_cod_b_range_indexers_pandas.json`.
- Retry predicate if rejected: only revisit if a same-host benchmark shows these
  vectorized RangeIndex indexers above 0.1% self-time and allocation profiling
  confirms residual output allocation or pandas-style vectorization, not
  miss-error construction, is still material.

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

## 2026-06-19 - br-frankenpandas-jlv2o - RangeIndex asof closed-form search

- Status: measured; keep.
- Lever: route ascending `RangeIndex::asof(IndexLabel::Int64)` through
  `searchsorted(..., "right") - 1` and direct `value_at`, avoiding the previous
  per-label scan across the whole range.
- Baseline comparator: current direct-label scan in `RangeIndex::asof`, which is
  allocation-free but still O(n) for every ascending lookup.
- Graveyard mapping: learned-index-style closed-form positioning over an affine
  key domain; use `(start, step, len)` as the semantic witness and binary-search
  the implicit arithmetic progression instead of materializing or scanning it.
- Alien-artifact proof obligation: `asof` keeps pandas-style
  preceding-or-equal semantics for ascending integer ranges, returns `None`
  before the first label and for empty ranges, and leaves descending/non-Int64
  behavior on the existing fallback path.
- Guard added:
  `range_index_asof_uses_closed_form_for_ascending_i64_jlv2o`, comparing
  ascending results with the flat-index oracle and covering singleton,
  descending, and empty ranges.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo check -p fp-index`
  on 2026-06-19; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- Benchmark verdict: accepted. Focused local Criterion plus pandas 2.2.3
  public-API timing measured 4,096 deterministic scalar probes over ascending
  ranges. `rch exec -- cargo bench -p fp-index --bench range_index_asof` also
  passed as the offloaded compile/bench guard; the ratio rows use local
  same-host Criterion/pandas measurements because the `hz2` worker did not have
  pandas installed.

| Rows | FP median | pandas median | Ratio vs pandas | Verdict | Artifact |
|---:|---:|---:|---:|---|---|
| 100k | 60.42 µs | 232.02 ms | 3,840x faster | KEEP | `artifacts/bench/gauntlet_cod_b_range_asof_vs_pandas.json` |
| 1M | 65.52 µs | 1,050.29 ms | 16,031x faster | KEEP | `artifacts/bench/gauntlet_cod_b_range_asof_vs_pandas.json` |

- Retry predicate if rejected: only revisit this family if a same-host profile
  shows `RangeIndex::asof` above 0.1% self-time and the residual cost is lookup
  positioning rather than caller-side alignment or scalar dispatch.
