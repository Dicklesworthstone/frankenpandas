# DOC-PASS-06: Concurrency/Lifecycle Semantics and Ordering Guarantees

**Bead:** bd-2gi.23.7
**Status:** Complete
**Date:** 2026-02-14
**Source Trees:** `legacy_pandas_code/pandas/pandas/` and `crates/fp-*/src/lib.rs`

---

## Table of Contents

1. [Summary](#1-summary)
2. [Pandas Concurrency Model](#2-pandas-concurrency-model)
3. [Pandas Ordering Guarantees](#3-pandas-ordering-guarantees)
4. [FrankenPandas Concurrency Model](#4-frankenpandas-concurrency-model)
5. [FrankenPandas Ordering Guarantees](#5-frankenpandas-ordering-guarantees)
6. [Race-Sensitive Behavioral Edges](#6-race-sensitive-behavioral-edges)
7. [Lifecycle State Machines](#7-lifecycle-state-machines)
8. [Cross-Cutting Ordering Invariants](#8-cross-cutting-ordering-invariants)
9. [Divergences from Pandas](#9-divergences-from-pandas)
10. [Appendix A: File Reference Index](#10-appendix-a-file-reference-index)

---

## 1. Summary

This document captures the thread/task/event lifecycle semantics, ordering guarantees, and
race-sensitive behavior for both legacy pandas (as captured in `legacy_pandas_code/pandas/`)
and the FrankenPandas (FP) Rust reimplementation (in `crates/fp-*/`).

**Scope:** Every claim about ordering, concurrency, and lifecycle is traced to specific
source file and line references. The document covers:

- How pandas manages (and fails to manage) concurrent access, mutation, and copy semantics.
- How FP's Rust ownership model eliminates entire classes of concurrency bugs by design.
- Ordering guarantees for index alignment, groupby, join, and IO operations.
- Lifecycle state machines for major operations in both systems.
- Cross-cutting invariants that FP code must maintain to be semantically compatible with pandas.

**Audience:** Developers working on FP crate implementations, conformance harness authors,
and anyone auditing FP for behavioral parity with pandas.

---

## 2. Pandas Concurrency Model

### 2.1 GIL Interactions and Thread Safety Guarantees

Pandas is a Python library and inherits CPython's Global Interpreter Lock (GIL). The GIL
provides the following guarantees:

- **Single-threaded Python bytecode execution:** Only one thread executes Python bytecode
  at a time. This prevents data races at the Python object level.
- **No GIL protection for C extensions:** When pandas calls into NumPy C code, Cython
  kernels, or other C extensions, those extensions may release the GIL. During GIL release,
  other Python threads may execute, creating potential for data races on shared numpy arrays.
- **Practical thread safety:** Pandas is NOT thread-safe for concurrent mutations. The
  official documentation states that pandas objects should not be shared across threads
  for writing. Read-only sharing is generally safe under the GIL, but no formal guarantees
  exist.

**Key source evidence:**
- `core/generic.py` uses `inplace=True` patterns (lines 1145, 1258, 1838, 1842, 1856)
  that mutate the object in place, which is fundamentally unsafe under concurrent access.
- `core/internals/managers.py` manages `BlockManager` state with mutable consolidation
  (line 2441 per DOC_COMPLEXITY_PERF.md), which can race with concurrent reads.
- Cython groupby kernels in `_libs/groupby.pyx` release the GIL during computation;
  a concurrent mutation of the source DataFrame could corrupt the kernel's input.

### 2.2 Copy-on-Write (CoW) Semantics

Pandas 2.x introduced Copy-on-Write (CoW) as an opt-in and later default behavior:

- **Pre-CoW behavior:** Operations like `df['col']` return a **view** (a reference to the
  same underlying memory) when the column's dtype allows it. Modifications to the view
  propagate to the parent DataFrame, creating the `SettingWithCopyWarning` trap.
- **CoW behavior:** Under CoW, every modification triggers a copy. Views are read-only;
  any attempt to write to a view creates a private copy first.
- **Consolidation interaction:** CoW complicates the `BlockManager` consolidation lifecycle
  because consolidation merges same-dtype blocks into contiguous arrays. Under CoW, these
  consolidated arrays must track reference counts to know whether a copy is needed on write.

**Source references:**
- `core/internals/managers.py:680` contains a `TODO(CoW)` comment about shallow copies.
- `core/internals/managers.py:820` has `TODO(CoW)` about handling CoW in `_consolidate`.
- `core/internals/managers.py:1218` notes "making arrays read-only might make this safer."
- `core/internals/managers.py:1507` documents that `setitem` would perform CoW again.
- `core/series.py:4396,5286,5367,5458,5715,6737,6810` all reference CoW documentation.
- `core/reshape/concat.py:223` and `core/reshape/merge.py:241` reference CoW behavior.

### 2.3 Mutation During Iteration

Pandas does NOT guarantee safe behavior when a DataFrame or Series is mutated during
iteration. Specifically:

- **`df.iterrows()` / `df.itertuples()`:** Iterating while another thread or the same
  code path mutates the DataFrame can produce inconsistent results, skip rows, or raise
  exceptions. The iterator captures the row count at creation but reads live data.
- **`BlockManager` consolidation during iteration:** If a mutation triggers consolidation
  between iteration steps, the internal block layout changes, potentially invalidating
  the iterator's position tracking.
- **GroupBy iteration:** `GroupBy.__iter__()` materializes group indices lazily. Mutating
  the source DataFrame during groupby iteration produces undefined behavior.

### 2.4 Chained Assignment Pitfalls

Chained assignment is the pattern `df['a']['b'] = value` (two successive `__getitem__`
and `__setitem__` calls). This is problematic because:

1. `df['a']` may return a view or a copy depending on dtype and BlockManager state.
2. If it returns a view, `['b'] = value` modifies the original DataFrame.
3. If it returns a copy, `['b'] = value` modifies a temporary that is immediately discarded.
4. Whether a view or copy is returned depends on the consolidation state of the BlockManager.

**Source reference:**
- `core/indexing.py:935` shows `iloc._setitem_with_indexer(indexer, value, self.name)`.
- `core/indexing.py:2370-2434` implements `_setitem_with_indexer` which handles the
  complex branching between in-place mutation and copy-then-set.
- `core/indexing.py:2322` documents the cases that can go through `DataFrame.__setitem__`.

### 2.5 BlockManager Consolidation Lifecycle

The `BlockManager` is the internal storage engine for pandas DataFrames:

1. **Initial state:** Columns are stored as separate `Block` objects, each holding a 2D
   numpy array for a group of same-dtype columns.
2. **Unconsolidated state:** After column additions or modifications, the BlockManager may
   contain multiple blocks of the same dtype. This is the "unconsolidated" state.
3. **Consolidation trigger:** Consolidation is triggered lazily on the next operation that
   requires a consolidated view (e.g., `values`, arithmetic, serialization). It merges
   same-dtype blocks into single contiguous arrays.
4. **Post-consolidation:** Each dtype has exactly one Block containing all columns of that
   type. Block positions are recomputed. This is an O(m log m + n*m) operation.

**Source references:**
- `core/internals/managers.py` implements `_consolidate()` (referenced at line 2441
  per DOC_COMPLEXITY_PERF.md section 1.1).
- `core/internals/construction.py:96` (`arrays_to_mgr`) is the entry point for
  constructing a BlockManager from arrays; consolidation may occur here.
- `core/internals/construction.py:375-460` (`dict_to_mgr`) handles the most common
  DataFrame construction path and terminates in `arrays_to_mgr`.

**Lifecycle diagram:**

```
[Unconsolidated] -- operation requiring consolidated view --> [Consolidating]
     ^                                                             |
     |                     [Consolidated]  <-----------------------+
     |                          |
     +--- column add/remove ----+
```

---

## 3. Pandas Ordering Guarantees

### 3.1 Index Label Ordering in Set Operations

Pandas provides the following ordering guarantees for index set operations:

| Operation | Ordering Guarantee | Source |
|-----------|-------------------|--------|
| `Index.union(other)` | Sorted if both inputs are sorted and compatible; otherwise follows left-first ordering. With `sort=None`, attempts to sort but does not guarantee it. | `core/indexes/base.py:2968` |
| `Index.intersection(other)` | By default unsorted (`sort=False`). When `sort=True`, sorted ascending. | `core/indexes/base.py:3186` |
| `union_indexes(indexes, sort)` | When `sort=True` (default), the union is sorted. When `sort=False`, preserves encounter order. | `core/indexes/api.py:185` |

**Key detail:** The `sort` parameter controls whether the result is lexicographically sorted.
When `sort=None`, pandas attempts to sort but falls back to the natural order if sorting
fails (e.g., mixed types). This is a source of ordering non-determinism.

### 3.2 GroupBy Key Ordering

Pandas groupby has two ordering modes controlled by the `sort` parameter:

- **`sort=True` (default):** Group keys are sorted lexicographically in the output. This
  is implemented via `groupsort_indexer` in `_libs/algos.pyx`, which produces a sorted
  permutation of group codes.
- **`sort=False`:** Group keys appear in first-seen (encounter) order. This is implemented
  via `factorize(val, sort=False)` at `core/groupby/generic.py:912`, which assigns codes
  in the order groups are first encountered.

**Source references:**
- `core/groupby/groupby.py:1672-1673` documents: "this is currently implementing sort=False
  (though the default is sort=True) for groupby in general."
- `core/groupby/categorical.py:67` documents: "sort=False should order groups in
  as-encountered order (GH-8868)."
- `core/groupby/ops.py:855` uses `compress_group_index(ids, sort=False)` for unsorted.
- `core/groupby/ops.py:890` uses `get_group_index(codes, shape, sort=True, xnull=True)`.

### 3.3 Concat/Merge Result Ordering

**`pd.concat()`:**
- Row ordering: Rows appear in the order of the input DataFrames/Series, first to last.
- Column ordering: Uses the union of all columns. By default, column order follows the
  first DataFrame, then appends new columns from subsequent DataFrames.
- Index ordering: Indexes are concatenated verbatim (duplicates preserved).

**`pd.merge()`:**
- Row ordering: For inner/left joins, rows follow the order of the left DataFrame. For
  right/outer joins, rows follow a more complex pattern preserving the "driving" table's
  order. Hash join (the default) preserves left-side row order for matched rows.
- Column ordering: Key column(s) first, then left non-key columns in their original order,
  then right non-key columns in their original order. Conflicts get `_x`/`_y` suffixes.

**Source references:**
- `core/reshape/concat.py:223` implements concat with configurable `sort` parameter.
- `core/reshape/merge.py:241` implements merge with `sort=False` by default (hash join).
- `core/reshape/merge.py:2103` sort-merge join path (requires sorted keys).
- `core/reshape/merge.py:2121` hash join path (preserves left order).

### 3.4 Column Ordering Preservation

Pandas column ordering is dict-based:

- **Construction:** `pd.DataFrame(dict)` uses Python dict insertion order (guaranteed since
  Python 3.7). The columns appear in the order they were inserted into the dict.
- **BlockManager impact:** Internally, columns are grouped into Blocks by dtype. The
  `BlockManager` maintains a mapping from column position to block+location. The user-facing
  column order is preserved separately from the block layout.
- **After operations:** Most pandas operations preserve column order. Operations like
  `df.sort_values()` preserve column order (only row order changes). `df.reindex(columns=...)`
  explicitly reorders columns.

---

## 4. FrankenPandas Concurrency Model

### 4.1 Single-Threaded Execution Model

FrankenPandas operates in a strictly single-threaded model with the following properties:

- **No GIL needed:** As a Rust library, FP uses Rust's ownership system instead of a GIL.
  All data structures are owned by exactly one binding at a time.
- **`#![forbid(unsafe_code)]`:** Every FP crate enforces `#![forbid(unsafe_code)]` at the
  crate level. This means:
  - No raw pointer dereferences.
  - No `unsafe` blocks, FFI calls, or transmutes.
  - No data races are possible (Rust's type system prevents `Send`/`Sync` violations).
  - No undefined behavior from memory safety violations.

**Source evidence:**
- `crates/fp-columnar/src/lib.rs:1`: `#![forbid(unsafe_code)]`
- `crates/fp-index/src/lib.rs:1`: `#![forbid(unsafe_code)]`
- `crates/fp-frame/src/lib.rs:1`: `#![forbid(unsafe_code)]`
- `crates/fp-types/src/lib.rs:1`: `#![forbid(unsafe_code)]`
- `crates/fp-groupby/src/lib.rs:1`: `#![forbid(unsafe_code)]`
- `crates/fp-join/src/lib.rs:1`: `#![forbid(unsafe_code)]`
- `crates/fp-io/src/lib.rs:1`: `#![forbid(unsafe_code)]`
- `crates/fp-runtime/src/lib.rs:1`: `#![forbid(unsafe_code)]`
- `crates/fp-expr/src/lib.rs:1`: `#![forbid(unsafe_code)]`
- `crates/fp-conformance/src/lib.rs:1`: `#![forbid(unsafe_code)]`

### 4.2 Arena Allocation Lifecycle

FP uses the `bumpalo` crate for arena-based allocation in performance-critical paths.
The arena lifecycle follows a strict pattern:

**Phase 1: Arena Creation**
- A `Bump` allocator is created at the entry of the operation.
- Budget check: `estimated_bytes <= arena_budget_bytes` (default 256 MiB).
- If budget is exceeded, the operation falls back to the global allocator.

**Phase 2: Arena Use**
- Intermediate data structures (position vectors, ordering vectors, accumulators) are
  allocated within the arena using `BumpVec<T>`.
- Arena allocations are O(1) amortized (bump pointer advancement).
- No individual deallocation occurs during this phase.

**Phase 3: Output Copy**
- Final results (output index labels, output scalar values) are **copied out** of the
  arena into globally-allocated `Vec<T>` structures.
- This copy is essential: the arena will be dropped, invalidating all arena-backed memory.

**Phase 4: Arena Drop**
- When the arena goes out of scope, all arena memory is freed in O(1) (single deallocation
  of the arena's backing chunks).
- Any references into the arena become dangling (but Rust's borrow checker prevents this
  at compile time).

**Source references:**
- `crates/fp-groupby/src/lib.rs:34`: `DEFAULT_ARENA_BUDGET_BYTES = 256 * 1024 * 1024`
- `crates/fp-groupby/src/lib.rs:193-239`: `groupby_sum_with_arena` creates arena at
  line 210, uses `BumpVec` for ordering at line 211, copies results out at lines 429-436.
- `crates/fp-groupby/src/lib.rs:381-439`: `try_groupby_sum_dense_int64_arena` creates
  arena at line 398, allocates sums/seen/ordering in arena at lines 400-404, copies out
  at lines 429-436.
- `crates/fp-join/src/lib.rs:34`: `DEFAULT_ARENA_BUDGET_BYTES = 256 * 1024 * 1024`
- `crates/fp-join/src/lib.rs:259-332`: `join_series_with_arena` creates arena at line 267,
  allocates position vectors in arena at lines 269-270, copies results out via
  `reindex_by_positions` at lines 320-325.

**Lifecycle diagram:**

```
[Budget Check]
    |
    +-- budget exceeded --> [Global Allocator Path]
    |
    +-- budget OK --> [Arena Created (Bump::new())]
                          |
                     [Arena Use: BumpVec allocations]
                          |
                     [Output Copy: Vec::with_capacity + copy]
                          |
                     [Arena Dropped: O(1) bulk free]
```

### 4.3 OnceCell Memoization Patterns

FP uses `std::cell::OnceCell` for lazy, cached computation of derived properties.
The pattern is: compute once, cache forever, never invalidate.

**`has_duplicates` cache:**
- **Location:** `crates/fp-index/src/lib.rs:111`: `duplicate_cache: OnceCell<bool>`
- **Computation:** `crates/fp-index/src/lib.rs:171-175`: `has_duplicates()` calls
  `self.duplicate_cache.get_or_init(|| detect_duplicates(&self.labels))`.
- **`detect_duplicates` implementation:** `crates/fp-index/src/lib.rs:125-133` builds a
  `HashMap<&IndexLabel, ()>` and checks for insertion collisions. O(n) time, O(n) space.
- **Invalidation:** None. The `Index` is immutable after construction, so the cache is
  always valid. There is no `set_labels` or `push_label` method.

**`is_sorted` cache (sort order detection):**
- **Location:** `crates/fp-index/src/lib.rs:114`: `sort_order_cache: OnceCell<SortOrder>`
- **Computation:** `crates/fp-index/src/lib.rs:177-183`: `sort_order()` calls
  `self.sort_order_cache.get_or_init(|| detect_sort_order(&self.labels))`.
- **`detect_sort_order` implementation:** `crates/fp-index/src/lib.rs:59-98` checks for
  `AscendingInt64`, `AscendingUtf8`, or `Unsorted` by scanning all label pairs.
- **Invalidation:** None. Same reasoning as above.

**Serde interaction:**
- Both caches are marked `#[serde(skip)]` at lines 110 and 113, so they are not serialized.
  After deserialization, the `OnceCell` is empty and will be recomputed on first access.

### 4.4 Column Immutability Model

FP columns are immutable after construction:

- **`Column` struct:** `crates/fp-columnar/src/lib.rs:441-446` defines `Column` with
  `dtype: DType`, `values: Vec<Scalar>`, and `validity: ValidityMask`. All fields are
  private (no `pub` on struct fields).
- **No mutation methods:** The `Column` API provides only `values()` (returns `&[Scalar]`),
  `value(idx)` (returns `Option<&Scalar>`), and constructors. There is no `set_value`,
  `push`, or `mutate` method.
- **Operations return new columns:** `binary_numeric`, `binary_comparison`, `filter_by_mask`,
  `fillna`, `dropna`, and `reindex_by_positions` all return `Result<Self, ColumnError>`,
  producing a new `Column` rather than modifying the existing one.

This immutability model eliminates:
- View vs. copy ambiguity (there are no views).
- `SettingWithCopyWarning` scenarios (there is no setting).
- Concurrent modification during iteration (there is no modification).

### 4.5 Series/DataFrame Value Semantics

FP Series and DataFrames use value semantics:

- **`Series`:** `crates/fp-frame/src/lib.rs:28-33` is `#[derive(Debug, Clone, PartialEq)]`.
  Cloning produces an independent copy. All operations (add, sub, mul, div, filter, etc.)
  return new `Series` values.
- **`DataFrame`:** `crates/fp-frame/src/lib.rs:616-620` uses `BTreeMap<String, Column>` for
  columns. It is `#[derive(Debug, Clone, PartialEq)]`. Operations like `with_column`,
  `drop_column`, `filter_rows`, `head`, `tail` all return new `DataFrame` values.
- **No `inplace` parameter:** No FP operation has an `inplace` mode. Every transformation
  produces a new value. This eliminates the entire class of inplace-mutation hazards.

---

## 5. FrankenPandas Ordering Guarantees

### 5.1 align_union Left-Order-First Guarantee

The `align_union` function guarantees that the output index contains:
1. All left labels in their original order, followed by
2. Right-only labels (those not present in the left index) in their original order.

**Implementation:** `crates/fp-index/src/lib.rs:519-546`

```rust
pub fn align_union(left: &Index, right: &Index) -> AlignmentPlan {
    let left_positions_map = left.position_map_first_ref();  // line 520
    let right_positions_map = right.position_map_first_ref(); // line 521

    let mut union_labels = Vec::with_capacity(left.labels.len() + right.labels.len());
    union_labels.extend(left.labels.iter().cloned());         // line 524: all left labels first
    for label in &right.labels {                              // line 525-528: right-only appended
        if !left_positions_map.contains_key(&label) {
            union_labels.push(label.clone());
        }
    }
    // ... position vector construction ...
}
```

**Invariant:** `INV-ALIGN-LEFT-FIRST` -- For `align_union(L, R)`, the output index is
`L ++ (R \ L)` where `\\` denotes set difference preserving right-side ordering.

**Test evidence:** `crates/fp-index/src/lib.rs:741-758`
`union_alignment_preserves_left_then_right_unseen_order` verifies that left=[1,2,4],
right=[2,3,4] produces union=[1,2,4,3] (left labels first, then right-only label 3).

### 5.2 align_inner Left-Order Preservation

The `align_inner` function preserves left-side ordering for the intersection:

**Implementation:** `crates/fp-index/src/lib.rs:478-498`

The output iterates over left labels and includes only those present in the right index.
The output order follows the left index exactly, skipping non-overlapping labels.

**Invariant:** `INV-ALIGN-INNER-LEFT` -- For `align_inner(L, R)`, the output index preserves
the relative order of labels from L that are also in R.

### 5.3 align_left Full Left Preservation

The `align_left` function preserves the entire left index as the output:

**Implementation:** `crates/fp-index/src/lib.rs:501-517`

The union index is simply a clone of the left index. Right positions are looked up via
`right_map.get(label).copied()`, yielding `None` for labels absent from the right.

### 5.4 GroupBy First-Seen Key Ordering

FP's groupby always produces keys in first-seen (encounter) order:

**Generic path:** `crates/fp-groupby/src/lib.rs:163-190` (global allocator variant) and
`crates/fp-groupby/src/lib.rs:211-238` (arena variant) both maintain an `ordering`
vector that records each new group key as it is first encountered:

```rust
let entry = slot.entry(key_id.clone()).or_insert_with(|| {
    ordering.push(key_id.clone());  // Record first-seen order
    (pos, 0.0)
});
```

**Dense Int64 path:** `crates/fp-groupby/src/lib.rs:324-377` (global) and
`crates/fp-groupby/src/lib.rs:381-439` (arena) maintain an `ordering: Vec<i64>` (or
`BumpVec<i64>`) that tracks first-seen key order:

```rust
if !seen[bucket] {
    seen[bucket] = true;
    ordering.push(key);  // First-seen order
}
```

**Invariant:** `INV-GROUPBY-FIRST-SEEN` -- For `groupby_sum(keys, values)`, the output
index labels appear in the order their corresponding key values were first encountered
during the left-to-right scan of the aligned key Series.

**Note:** This corresponds to pandas' `sort=False` behavior. Pandas defaults to
`sort=True` (lexicographic key sorting). FP does NOT sort keys by default. This is a
documented divergence (see Section 9).

### 5.5 Leapfrog Triejoin Ordering Properties

The leapfrog operations produce **sorted** output (unlike pairwise `align_union`):

**`leapfrog_union`:** `crates/fp-index/src/lib.rs:572-619`
- Each input is sorted and deduplicated before merging.
- A min-heap produces labels in ascending sorted order.
- The output is a sorted, deduplicated index.

**`leapfrog_intersection`:** `crates/fp-index/src/lib.rs:626-696`
- Each input is sorted and deduplicated before intersection.
- The classic leapfrog algorithm advances cursors in sorted order.
- The output is a sorted, deduplicated index.

**`multi_way_align`:** `crates/fp-index/src/lib.rs:703-734`
- Uses `leapfrog_union` for the union index, so the output is sorted.
- Position vectors map sorted union labels to original positions.

**Invariant:** `INV-LEAPFROG-SORTED` -- All leapfrog operations produce sorted, deduplicated
output regardless of input order.

**Test evidence:** `crates/fp-index/src/lib.rs:1260-1276` verifies that
`leapfrog_union(&[a, b, c])` with a=[1,3,5], b=[2,3,6], c=[4,5,6] produces [1,2,3,4,5,6].

**Isomorphism test:** `crates/fp-index/src/lib.rs:1413-1428` verifies that the leapfrog
union produces the same label set as iterative pairwise union (after sorting).

### 5.6 Join Output Ordering

FP's join operations produce output in the following order:

**Inner join:** `crates/fp-join/src/lib.rs:198-257` (global) and lines 272-332 (arena):
- Iterates left labels in order.
- For each left label, emits all matching right positions in their stored order.
- Output follows left-side row order for the driving table.

**Left join:** Same as Inner, except unmatched left rows are also emitted (with `None`
for right positions). Left-side order is fully preserved.

**Right join:** `crates/fp-join/src/lib.rs:229-246`:
- Iterates right labels in order.
- For each right label, emits all matching left positions.
- Unmatched right rows are emitted with `None` for left positions.
- Output follows right-side row order.

**Outer join:** `crates/fp-join/src/lib.rs:218-227`:
- First, emits all left-matched and left-unmatched rows (same as Left join).
- Then, appends right-only rows (right labels not present in left index).
- Output is left-rows-first, then right-only rows.

**Invariant:** `INV-JOIN-LEFT-ORDER` -- For Inner and Left joins, the output preserves the
left table's row order for matched rows. For Right joins, the right table's order is
preserved. For Outer joins, left rows appear first, followed by right-only rows.

**DataFrame merge:** `crates/fp-join/src/lib.rs:360-502` (`merge_dataframes`) follows the
same ordering logic as Series-level joins. Column ordering in the merged output uses
`BTreeMap<String, Column>`, which sorts column names lexicographically. Key column is
inserted first at line 474, then left non-key columns, then right non-key columns.

### 5.7 CSV Column Ordering

FP's CSV reader produces columns in header order (insertion order):

**Implementation:** `crates/fp-io/src/lib.rs:59-96` (`read_csv_str`):
- Line 70-76: AG-07 optimization uses `Vec<Vec<Scalar>>` for column accumulation.
  Columns are indexed by their position in the header row.
- Line 88-92: Columns are inserted into a `BTreeMap` in header order. Since BTreeMap
  sorts by key, the output column order is **alphabetically sorted**, not header order.

**Invariant:** `INV-CSV-INSERT-ORDER` -- CSV column names are stored in a `BTreeMap`, so
they appear in **lexicographic order** regardless of their position in the CSV header.
This differs from pandas, which preserves header order. (See Section 9 for divergence.)

**JSON reader:** `crates/fp-io/src/lib.rs:291-410` similarly uses `BTreeMap` for column
storage, producing lexicographic column ordering.

### 5.8 Index Set Operation Ordering

FP index set operations have the following ordering properties:

| Operation | Ordering | Source |
|-----------|----------|--------|
| `union_with` | Left-first, then right-only, both in original order | `fp-index/src/lib.rs:342-351` |
| `intersection` | Left-order for labels present in both | `fp-index/src/lib.rs:329-339` |
| `difference` | Left-order for labels not in other | `fp-index/src/lib.rs:354-364` |
| `symmetric_difference` | Left-only (in left order) then right-only (in right order) | `fp-index/src/lib.rs:367-383` |
| `unique` | First-seen order (preserves encounter order) | `fp-index/src/lib.rs:275-283` |

**Test evidence:** `crates/fp-index/src/lib.rs:1044-1053` verifies `unique()` preserves
first-seen order: input ["b","a","b","c","a"] yields ["b","a","c"].

---

## 6. Race-Sensitive Behavioral Edges

### 6.1 Pandas: SettingWithCopyWarning Scenarios

The `SettingWithCopyWarning` arises when pandas cannot determine whether a mutation
operates on a view or a copy. Common triggers:

**Scenario 1: Boolean-indexed slice assignment**
```python
df[df['a'] > 0]['b'] = 5  # Modifies a copy! Original df unchanged.
```
The `df[df['a'] > 0]` creates a copy (because boolean indexing always copies), so the
subsequent `['b'] = 5` modifies the copy, not the original.

**Scenario 2: Column-then-row chained access**
```python
df['col1']['row_label'] = value  # May modify original OR a copy
```
Whether `df['col1']` returns a view depends on the BlockManager consolidation state.

**Scenario 3: `loc` with non-contiguous indexer**
```python
df.loc[[1, 3, 5], 'col'] = value  # Fancy indexing may create a copy
```

**Source references:**
- `core/indexing.py:2370-2434`: `_setitem_with_indexer` contains the branching logic.
- `core/indexing.py:2438-2527`: `_setitem_with_indexer_split_path` handles the split case.

### 6.2 Pandas: inplace=True Mutation During View Aliasing

When `inplace=True` is used on a DataFrame that shares memory with another DataFrame
(view aliasing), the mutation affects both the original and the alias:

```python
df2 = df[['a', 'b']]  # df2 may be a view into df
df2.fillna(0, inplace=True)  # May modify df's underlying memory!
```

Under CoW, `inplace=True` first creates a private copy before mutating, breaking the
alias. Without CoW, this is a genuine data race under concurrent access.

**Source reference:** `core/generic.py:1145` documents the `inplace=True` return convention.

### 6.3 FP: OnceCell Cache Invalidation

FP uses `OnceCell` for lazy caches (`has_duplicates`, `is_sorted`). The key property is:

- **No invalidation needed:** `Index` is immutable after construction. The `labels` vector
  is private and has no mutation methods. Therefore, the cached values cannot become stale.
- **Serde bypass:** After deserialization, the `OnceCell` is empty (`#[serde(skip)]` at
  `fp-index/src/lib.rs:110,113`). The first access recomputes the cached value. This is
  correct because the labels are fully reconstructed during deserialization.
- **Clone behavior:** `OnceCell` does not implement `Clone` in a way that preserves the
  cached value in older Rust editions. The `Index` struct derives `Clone` via
  `#[derive(Debug, Clone)]` at line 107, and `OnceCell` is `Clone` (cloning an initialized
  `OnceCell` preserves the value). However, the `Serialize/Deserialize` derive with
  `#[serde(skip)]` means the cache is always recomputed after deserialization.

**Edge case:** If an `Index` is cloned and the original had a computed cache, the clone
will share the same cache value. This is correct because the labels are identical.

### 6.4 FP: Arena Lifetime vs Output Lifetime

The critical invariant for arena-based operations:

**Invariant:** All output data must be copied from arena-backed storage into globally-allocated
storage BEFORE the arena is dropped.

**Correct pattern (groupby):** `crates/fp-groupby/src/lib.rs:429-436`:
```rust
// Copy results out of arena into global-allocated output.
let mut out_index = Vec::with_capacity(ordering.len());
let mut out_values = Vec::with_capacity(ordering.len());
for key in ordering.iter().copied() {
    // ... copy from arena-backed sums[] ...
    out_index.push(IndexLabel::Int64(key));
    out_values.push(Scalar::Float64(sums[bucket]));
}
```

**Correct pattern (join):** `crates/fp-join/src/lib.rs:320-325`:
```rust
let left_values = left.column()
    .reindex_by_positions(left_positions.as_slice())?;  // Copies into new Vec
let right_values = right.column()
    .reindex_by_positions(right_positions.as_slice())?;  // Copies into new Vec
```

The `reindex_by_positions` method copies data into a new `Vec<Scalar>` (owned by the
returned `Column`), so the output survives arena drop.

**Rust safety guarantee:** The borrow checker prevents returning references to arena-backed
data. `BumpVec<T>` borrows the `Bump` allocator, so it cannot outlive the arena. Any attempt
to return a `BumpVec` from a function that owns the `Bump` would fail to compile.

### 6.5 FP: Concurrent Conformance Harness Execution

The conformance harness (`crates/fp-conformance/src/lib.rs`) executes test fixtures
sequentially. There is no parallel fixture execution:

- `run_smoke` at line 87 reads the fixture directory and counts entries.
- Fixture execution follows a load-execute-compare-report cycle (see Section 7.5).
- The oracle subprocess (`python3`) is invoked synchronously via `std::process::Command`.

No concurrent access to shared state occurs during conformance runs.

---

## 7. Lifecycle State Machines

### 7.1 DataFrame Construction Lifecycle

**Pandas (`pd.DataFrame(data)`):**

```
[Input Data] --> [Type Dispatch] --> [dict_to_mgr / ndarray_to_mgr / arrays_to_mgr]
                                           |
                                     [Index Inference (_extract_index)]
                                           |
                                     [Block Formation (create_block_manager_from_blocks)]
                                           |
                                     [Consolidation (optional, lazy)]
                                           |
                                     [NDFrame.__init__(self, mgr)]
```

**FP (`DataFrame::new` / `DataFrame::from_dict`):**

```
[Input: BTreeMap<String, Column> + Index]
    |
    +-- [Length Validation: column.len() == index.len() for all columns]
    |     |
    |     +-- FAIL --> FrameError::LengthMismatch
    |
    +-- PASS --> [DataFrame { index, columns }]  (construction complete)
```

**FP (`DataFrame::from_series`):** `crates/fp-frame/src/lib.rs:639-665`

```
[Input: Vec<Series>]
    |
    +-- Phase 1: Compute global union index
    |     |
    |     for each series[1..]:
    |         union_index = align_union(union_index, series.index).union_index
    |
    +-- Phase 2: Reindex each column to union
    |     |
    |     for each series:
    |         aligned_column = series.column.reindex_by_positions(plan.right_positions)
    |         columns.insert(series.name, aligned_column)
    |
    +-- [DataFrame::new(union_index, columns)]
```

**Key difference:** Pandas construction involves complex type dispatch, index inference,
block formation, and optional lazy consolidation. FP construction is a simple validation
pass followed by struct initialization.

### 7.2 Series Alignment Lifecycle During Binary Ops

**Pandas (`s1 + s2`):**

```
[s1.__add__(s2)]
    |
    +-- [_arith_method(s2, op=operator.add)]
    |     |
    |     +-- [Identical index check: s1.index is s2.index]
    |     |     |
    |     |     +-- YES --> [Direct element-wise numpy add]
    |     |     +-- NO -->  [NDFrame.align(other, join='outer')]
    |     |                    |
    |     |                    +-- [Index.join(other.index)]
    |     |                    |     |
    |     |                    |     +-- [Sorted + unique: merge join O(n+m)]
    |     |                    |     +-- [Non-unique: hash join O(n+m) to O(n*m)]
    |     |                    |
    |     |                    +-- [Reindex both sides to union]
    |     |                    +-- [Element-wise numpy add]
    |     |
    |     +-- [Result dtype computation]
    |     +-- [NaN propagation]
    |     +-- [Construct result Series]
```

**FP (`s1.add(&s2)`):** `crates/fp-frame/src/lib.rs:107-153`

```
[s1.add(&s2)]
    |
    +-- [Duplicate index check: has_duplicates() for both]
    |     |
    |     +-- Strict mode + duplicates --> FrameError::DuplicateIndexUnsupported
    |
    +-- [align_union(&self.index, &other.index)] --> AlignmentPlan
    |     |
    |     +-- [validate_alignment_plan(&plan)]
    |
    +-- [left = self.column.reindex_by_positions(&plan.left_positions)]
    +-- [right = other.column.reindex_by_positions(&plan.right_positions)]
    |
    +-- [Runtime policy: decide_join_admission(plan.union_index.len())]
    |     |
    |     +-- Reject --> FrameError::CompatibilityRejected
    |
    +-- [column = left.binary_numeric(&right, ArithmeticOp::Add)]
    |     |
    |     +-- [Length check: left.len() == right.len()]
    |     +-- [Output dtype: common_dtype(left.dtype, right.dtype)]
    |     +-- [Try vectorized path (AG-10): try_vectorized_binary]
    |     |     |
    |     |     +-- Float64: vectorized_binary_f64 on &[f64] slices
    |     |     +-- Int64 (non-div): vectorized_binary_i64 on &[i64] slices
    |     |     +-- Other: fallback to scalar path
    |     |
    |     +-- [Scalar fallback: zip iterators, per-element arithmetic]
    |     +-- [NaN/Null propagation: NaN+valid=NaN, Null+valid=Null]
    |
    +-- [Series::new(out_name, plan.union_index, column)]
```

### 7.3 GroupBy Lifecycle

**Pandas (`df.groupby(keys).sum()`):**

```
[DataFrame.groupby(by=keys)]
    |
    +-- [get_grouper(): factorize keys --> codes, uniques]
    |     |
    |     +-- sort=True: sorted code assignment
    |     +-- sort=False: first-seen code assignment
    |
    +-- [BaseGrouper: stores codes, ngroups]
    |
    +-- [.sum() invocation]
    |     |
    |     +-- [_cython_agg_general: dispatch to Cython kernel]
    |     |     |
    |     |     +-- [group_sum in _libs/groupby.pyx: single pass, array-indexed accumulators]
    |     |
    |     +-- [_agg_py_fallback: for EA/object dtypes, per-group Python iteration]
    |     |
    |     +-- [Reconstruct result DataFrame with group keys as index]
    |     +-- [Optional sort: sort_index if sort=True]
```

**FP (`groupby_sum(keys, values, options)`):** `crates/fp-groupby/src/lib.rs:58-134`

```
[groupby_sum(keys, values, options)]
    |
    +-- [Alignment Check: keys.index() == values.index() && !has_duplicates()]
    |     |
    |     +-- Match: skip alignment (identity fast path)
    |     +-- No match: align_union(keys.index(), values.index())
    |                    reindex both key and value columns
    |
    +-- [Budget Estimation: estimate_groupby_intermediate_bytes(input_rows)]
    |
    +-- [Arena Decision: estimated_bytes <= arena_budget_bytes?]
    |     |
    |     +-- YES --> groupby_sum_with_arena path
    |     +-- NO  --> groupby_sum_with_global_allocator path
    |
    +-- [Dense Int64 Path Attempt: try_groupby_sum_dense_int64[_arena]]
    |     |
    |     +-- All keys Int64, span <= 65536 --> Dense bucket accumulation
    |     |     |
    |     |     +-- [Allocate sums[bucket_len], seen[bucket_len], ordering[]]
    |     |     +-- [Single pass: bucket = key - min_key, sums[bucket] += value]
    |     |     +-- [Emit in first-seen order via ordering vector]
    |     |
    |     +-- Not all Int64 or span too large --> Fall through to generic path
    |
    +-- [Generic HashMap Path]
    |     |
    |     +-- [ordering: Vec<GroupKeyRef> or BumpVec<GroupKeyRef>]
    |     +-- [slot: HashMap<GroupKeyRef, (usize, f64)>]
    |     +-- [Single pass: insert to ordering on first seen, accumulate sum]
    |     +-- [emit_groupby_result: iterate ordering, reconstruct IndexLabel from source]
    |
    +-- [Series::new("sum", Index::new(out_index), out_column)]
```

### 7.4 Join Lifecycle

**Pandas (`pd.merge(left, right, on=key, how='inner')`):**

```
[merge(left, right, on, how)]
    |
    +-- [_factorize_keys: build integer codes for join keys]
    |
    +-- [DISPATCH on method:]
    |     |
    |     +-- Hash join (_join_by_index or _join_compat):
    |     |     Build hash table from right keys --> probe with left keys
    |     |
    |     +-- Sort-merge join: binary search on sorted keys
    |
    +-- [_get_join_result: compute left_indexer, right_indexer]
    |
    +-- [_reindex_and_concat: reindex both sides, concatenate columns]
    |
    +-- [Result DataFrame construction]
```

**FP (`join_series(left, right, join_type)`):** `crates/fp-join/src/lib.rs:58-131`

```
[join_series(left, right, join_type)]
    |
    +-- [Build Phase: HashMap<&IndexLabel, Vec<usize>> from right index]
    |     (AG-02: borrowed-key HashMap eliminates label clones)
    |
    +-- [Optional: build left_map for Right/Outer joins]
    |
    +-- [Estimate output rows and intermediate bytes]
    |
    +-- [Arena Decision: estimated_bytes <= arena_budget_bytes?]
    |     |
    |     +-- YES --> join_series_with_arena
    |     +-- NO  --> join_series_with_global_allocator
    |
    +-- [Probe Phase (varies by join type):]
    |     |
    |     +-- Inner/Left/Outer: iterate left labels
    |     |     For each left label:
    |     |       Match in right_map? --> emit (left_pos, right_pos) pairs
    |     |       No match + Left/Outer? --> emit (left_pos, None)
    |     |     Outer: append right-only labels with (None, right_pos)
    |     |
    |     +-- Right: iterate right labels
    |     |     For each right label:
    |     |       Match in left_map? --> emit (left_pos, right_pos) pairs
    |     |       No match? --> emit (None, right_pos)
    |
    +-- [Emit Phase: reindex_by_positions for both columns]
    |
    +-- [JoinedSeries { index, left_values, right_values }]
```

**DataFrame merge:** `crates/fp-join/src/lib.rs:360-502` follows the same lifecycle but
additionally handles:
- Key column extraction from both DataFrames (lines 366-375).
- Scalar-to-IndexLabel conversion for hashing (line 343-351, `scalar_to_key`).
- Column conflict resolution with `_left`/`_right` suffixes (lines 477-500).
- Output index as auto-generated RangeIndex (line 466).

### 7.5 Conformance Harness Lifecycle

**FP Conformance Harness:** `crates/fp-conformance/src/lib.rs`

```
[HarnessConfig::default_paths()]
    |
    +-- [Load Fixtures: read YAML files from fixture_root]
    |     |
    |     +-- Deserialize FixturePacket from YAML
    |     +-- Extract: operation, input series, expected output
    |
    +-- [Execute FP Operation:]
    |     |
    |     +-- Match on FixtureOperation enum (line 123-151):
    |     |     SeriesAdd -> series.add(&other)
    |     |     SeriesJoin -> join_series(left, right, join_type)
    |     |     GroupBySum -> groupby_sum(keys, values, options)
    |     |     IndexAlignUnion -> align_union(left, right)
    |     |     etc.
    |
    +-- [Compare: semantic_eq on output vs expected]
    |     |
    |     +-- Index label comparison
    |     +-- Value comparison (with NaN-aware semantics)
    |     +-- Dtype comparison
    |
    +-- [Report: HarnessReport { suite, oracle_present, fixture_count, strict_mode }]
    |
    +-- [Optional Oracle: invoke pandas_oracle.py via subprocess]
    |     |
    |     +-- Pass fixture input as JSON to Python subprocess
    |     +-- Parse Python output as expected values
    |     +-- Compare FP output to oracle output
```

### 7.6 IVM (Incremental View Maintenance) Lifecycle

**FP IVM:** `crates/fp-expr/src/lib.rs:76-198`

```
[MaterializedView::from_full_eval(expr, context)]
    |
    +-- [Full evaluation: evaluate(expr, context) --> result Series]
    +-- [Snapshot: clone context as base_snapshot]
    |
    +-- [apply_delta(delta, context)]
         |
         +-- [Linearity check: is_linear(expr)?]
         |     |
         |     +-- YES (Series refs, Adds only):
         |     |     [evaluate_delta: compute only new rows]
         |     |     [concat_series([old_result, delta_result])]
         |     |     [Update result and base_snapshot]
         |     |
         |     +-- NO (Literals, non-linear):
         |           [Full re-evaluation: evaluate(expr, context)]
         |           [Replace result and base_snapshot]
```

**Ordering in IVM:** The concatenation `concat_series([old_result, delta_result])` appends
delta rows after existing rows, preserving temporal ordering. Old index labels come first,
delta labels come second.

---

## 8. Cross-Cutting Ordering Invariants

This section catalogs the formal ordering invariants that FP must maintain. Each invariant
is identified by a code, specified precisely, and traced to its implementation.

### INV-ALIGN-LEFT-FIRST

**Statement:** For `align_union(L, R)`, the output index is the concatenation of all labels
from L (in L's order) followed by labels from R that are not in L (in R's order).

**Formal:** `output = L ++ filter(R, lambda r: r not in set(L))`

**Implementation:** `crates/fp-index/src/lib.rs:519-546`
- Line 524: `union_labels.extend(left.labels.iter().cloned())` -- all of L in order.
- Lines 525-528: Iterate R, append only labels not in left_positions_map.

**Test:** `crates/fp-index/src/lib.rs:741-758`

### INV-GROUPBY-FIRST-SEEN

**Statement:** For `groupby_sum(keys, values)` and all `groupby_agg` operations, the output
index labels appear in the order their key values were first encountered during the
left-to-right scan of the aligned key column.

**Formal:** `output_order = stable_unique(aligned_keys)`

**Implementation (dense):** `crates/fp-groupby/src/lib.rs:354-357`:
```rust
if !seen[bucket] {
    seen[bucket] = true;
    ordering.push(key);
}
```

**Implementation (generic):** `crates/fp-groupby/src/lib.rs:176-178`:
```rust
let entry = slot.entry(key_id.clone()).or_insert_with(|| {
    ordering.push(key_id.clone());
    (pos, 0.0)
});
```

### INV-JOIN-LEFT-ORDER

**Statement:** For Inner and Left joins, the output rows preserve the row order of the left
table. For each left row with multiple matches in the right table, the matched right rows
appear in the order they occur in the right table's index.

**Formal:** Given left rows `[l0, l1, ..., ln]` and right index with matches, the output
for Inner/Left is: for each `li` in order, if `li` matches right rows `[rj, rk, ...]`,
emit `(li, rj), (li, rk), ...` in the order `j < k`.

**Implementation:** `crates/fp-join/src/lib.rs:201-216`:
```rust
for (left_pos, label) in left.index().labels().iter().enumerate() {
    if let Some(matches) = right_map.get(label) {
        for right_pos in matches {
            out_labels.push(label.clone());
            left_positions.push(Some(left_pos));
            right_positions.push(Some(*right_pos));
        }
    }
    // ...
}
```

The `right_map` stores positions in a `Vec<usize>` where positions are appended during
the build phase (`right_map.entry(label).or_default().push(pos)` at line 85), so they
appear in the order of the right table's index.

### INV-CSV-INSERT-ORDER

**Statement:** CSV (and JSON) column names in the output DataFrame are stored in a
`BTreeMap<String, Column>`, which sorts keys lexicographically.

**Implementation:** `crates/fp-io/src/lib.rs:88-92`:
```rust
let mut out_columns = BTreeMap::new();
for (idx, values) in columns.into_iter().enumerate() {
    let name = headers.get(idx).unwrap_or_default().to_owned();
    out_columns.insert(name, Column::from_values(values)?);
}
```

**Note:** This is a **divergence** from pandas, which preserves header order. FP uses
`BTreeMap` for deterministic ordering, but this means column order is alphabetical, not
positional.

### INV-CONCAT-APPEND

**Statement:** For `concat_series([s1, s2, ...])` and `concat_dataframes([df1, df2, ...])`,
index labels are concatenated in input order, preserving duplicates.

**Implementation:** `crates/fp-frame/src/lib.rs:543-570`:
```rust
for s in series_list {
    labels.extend_from_slice(s.index().labels());
    values.extend_from_slice(s.values());
}
```

### INV-LEAPFROG-SORTED

**Statement:** All leapfrog operations (union, intersection, multi_way_align) produce
sorted, deduplicated output indexes, regardless of input order.

**Implementation:** `crates/fp-index/src/lib.rs:572-619` (union uses min-heap for sorted
merge), `crates/fp-index/src/lib.rs:626-696` (intersection uses sorted cursor advancement).

### INV-INDEX-UNIQUE-FIRST-SEEN

**Statement:** `Index::unique()` returns labels in first-seen encounter order.

**Implementation:** `crates/fp-index/src/lib.rs:275-283`:
```rust
let mut seen = HashMap::<&IndexLabel, ()>::new();
let labels: Vec<IndexLabel> = self.labels.iter()
    .filter(|l| seen.insert(l, ()).is_none())
    .cloned()
    .collect();
```

### INV-IMMUTABILITY

**Statement:** All FP data structures (Column, Index, Series, DataFrame) are immutable after
construction. Every transformation produces a new value. There are no `inplace`, `set_value`,
or `mutate` methods.

**Evidence:** All public methods on Column, Index, Series, and DataFrame return new values
(`Result<Self, Error>`) or references (`&[Scalar]`, `&Index`). No method takes `&mut self`
except `CrackIndex` (which is explicitly a mutable adaptive index structure).

---

## 9. Divergences from Pandas

### 9.1 Ordering Divergences

| Aspect | Pandas Behavior | FP Behavior | Impact |
|--------|----------------|-------------|--------|
| GroupBy key order (default) | Sorted lexicographically (`sort=True`) | First-seen encounter order | FP matches pandas `sort=False` |
| `align_union` output order | Sorted if both inputs are sorted and of compatible types; otherwise left-first | Always left-first, then right-only | May differ when pandas sorts |
| CSV column order | Header order (insertion order) | Alphabetical (`BTreeMap` sorted) | Column order divergence |
| JSON column order | Insertion order | Alphabetical (`BTreeMap` sorted) | Column order divergence |
| DataFrame column storage | Dict (insertion order since Python 3.7) | `BTreeMap<String, Column>` (sorted) | Column iteration order differs |
| `Index.union()` | `sort=None` by default (attempts sort) | `union_with`: left-first (no sort) | Set operation order differs |
| `Index.intersection()` | `sort=False` by default | Left-order preserved | Compatible when pandas `sort=False` |
| Leapfrog multi-way align | N/A (no leapfrog in pandas) | Always sorted, deduplicated | Novel FP operation |

### 9.2 Lifecycle Divergences

| Aspect | Pandas Behavior | FP Behavior | Impact |
|--------|----------------|-------------|--------|
| Mutation model | Mutable objects, `inplace=True` supported | Immutable values, no `inplace` | Eliminates mutation bugs |
| Copy semantics | View/copy ambiguity, CoW opt-in | Always copy (value semantics) | No SettingWithCopyWarning |
| BlockManager consolidation | Lazy consolidation, affects view/copy behavior | No BlockManager, direct Column storage | Simpler mental model |
| Memory management | Python GC + reference counting | Rust ownership + optional arena allocation | Deterministic lifetimes |
| Thread safety | Not thread-safe for writes; GIL protects Python-level reads | Thread-safe by construction (`forbid(unsafe_code)`, ownership) | No data races possible |
| Duplicate index handling | Supported (with performance penalties) | Strict mode rejects; hardened mode warns | Different defaults |
| Null propagation in groupby keys | `dropna=True` by default | `dropna: true` by default | Compatible |

### 9.3 Semantic Divergences

| Aspect | Pandas Behavior | FP Behavior | Impact |
|--------|----------------|-------------|--------|
| Division type | `int / int` -> `float64` | Same: `ArithmeticOp::Div` always promotes to `Float64` | Compatible |
| NaN vs Null distinction | `np.nan` for float, `pd.NA` for nullable | `Scalar::Null(NullKind::NaN)` vs `Scalar::Null(NullKind::Null)` | FP preserves distinction |
| Integer overflow | Wraps (numpy behavior) | `wrapping_add/sub/mul` (same) | Compatible |
| Runtime policy | None (pandas has no admission control) | Bayesian decision engine with conformal calibration | Novel FP feature |
| Adaptive indexing | None | CrackIndex for progressive column partitioning | Novel FP feature |

---

## 10. Appendix A: File Reference Index

### 10.1 FrankenPandas Source Files

| File | Lines | Key Concepts |
|------|-------|-------------|
| `crates/fp-types/src/lib.rs` | ~210 | `Scalar`, `DType`, `NullKind`, `TypeError`, `common_dtype`, `cast_scalar_owned` |
| `crates/fp-columnar/src/lib.rs` | ~1927 | `ValidityMask` (packed bitvec), `Column`, `ColumnData`, `ArithmeticOp`, `ComparisonOp`, `CrackIndex`, vectorized binary ops |
| `crates/fp-index/src/lib.rs` | ~1613 | `Index`, `IndexLabel`, `AlignmentPlan`, `AlignMode`, `OnceCell` caches, `align_union`, `align_inner`, `align_left`, `leapfrog_union`, `leapfrog_intersection`, `multi_way_align` |
| `crates/fp-frame/src/lib.rs` | ~1000+ | `Series`, `DataFrame`, binary ops with policy, comparison ops, filter, concat, from_dict, from_series, `RuntimePolicy` integration |
| `crates/fp-groupby/src/lib.rs` | ~500+ | `groupby_sum` (global + arena), dense Int64 fast path, generic HashMap path, `groupby_agg`, `AggFunc`, `GroupKeyRef`, arena lifecycle |
| `crates/fp-join/src/lib.rs` | ~500+ | `join_series` (global + arena), `merge_dataframes`, `JoinType` (Inner/Left/Right/Outer), borrowed-key HashMap, output row estimation |
| `crates/fp-io/src/lib.rs` | ~487+ | `read_csv_str`, `write_csv_string`, `read_json_str`, `write_json_string`, CSV options, JSON orient modes, `BTreeMap` column storage |
| `crates/fp-expr/src/lib.rs` | ~200+ | `Expr` (Series/Add/Literal), `EvalContext`, `evaluate`, `MaterializedView`, `Delta`, incremental view maintenance |
| `crates/fp-runtime/src/lib.rs` | ~920 | `RuntimePolicy` (Strict/Hardened), `EvidenceLedger`, Bayesian decision engine, `ConformalGuard`, `RaptorQEnvelope` |
| `crates/fp-conformance/src/lib.rs` | ~200+ | `HarnessConfig`, `FixtureOperation`, conformance test runner, oracle integration |

### 10.2 Pandas Source Files

| File | Key Concepts for Concurrency/Lifecycle |
|------|---------------------------------------|
| `core/frame.py` | `DataFrame.__init__` (line 455), `_arith_method_with_reindex` (line 9065), `__setitem__` chained assignment |
| `core/generic.py` | `inplace=True` handling (lines 1145, 1258, 1838, 1842, 1856), `_reset_cache`, `_clear_item_cache` |
| `core/series.py` | CoW documentation (lines 4396, 5286, 5367, 5458, 5715, 6737, 6810) |
| `core/indexing.py` | `_setitem_with_indexer` (line 2370), chained assignment logic, `SettingWithCopy` scenarios |
| `core/internals/managers.py` | `BlockManager`, consolidation (line 2441), CoW TODOs (lines 680, 820, 1218, 1507) |
| `core/internals/construction.py` | `dict_to_mgr` (line 375), `ndarray_to_mgr` (line 193), `arrays_to_mgr` (line 96), `_extract_index` |
| `core/internals/blocks.py` | Block storage, dtype-homogeneous 2D arrays |
| `core/groupby/groupby.py` | `sort=True/False` (lines 1672-1673), `_cython_agg_general` (line 1510), `_agg_py_fallback` (line 1462) |
| `core/groupby/categorical.py` | `sort=False` first-seen ordering (line 67, GH-8868) |
| `core/groupby/generic.py` | `factorize(val, sort=False)` (line 912) |
| `core/groupby/ops.py` | `compress_group_index(ids, sort=False)` (line 855) |
| `core/indexes/base.py` | `Index.union` (line 2968), `Index.intersection` (line 3186) |
| `core/indexes/api.py` | `union_indexes` (line 185) |
| `core/reshape/merge.py` | Hash join (line 2121), sort-merge join (line 2103), `_reindex_and_concat` (line 1081) |
| `core/reshape/concat.py` | `pd.concat` implementation (line 223) |

### 10.3 Test File Reference

| Test File | Coverage |
|-----------|----------|
| `crates/fp-index/src/lib.rs` (mod tests) | Alignment ordering, OnceCell caching, leapfrog correctness, set operation ordering |
| `crates/fp-columnar/src/lib.rs` (mod tests) | ValidityMask packed bitvec, vectorized binary ops, NaN propagation, CrackIndex |
| `crates/fp-frame/src/lib.rs` (mod tests) | Series alignment in add, strict mode duplicate rejection, concat ordering |
| `crates/fp-conformance/tests/smoke.rs` | Harness lifecycle, fixture loading |
| `crates/fp-conformance/tests/proptest_properties.rs` | Property-based testing for alignment invariants |
| `crates/fp-conformance/tests/ag_e2e.rs` | End-to-end alignment-groupby-join pipeline tests |

---

*End of DOC-PASS-06: Concurrency/Lifecycle Semantics and Ordering Guarantees*
