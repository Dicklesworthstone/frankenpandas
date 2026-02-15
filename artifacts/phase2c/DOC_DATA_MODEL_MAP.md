# DOC-PASS-03: Data Model, State, and Invariant Mapping

**Bead**: bd-2gi.23.4
**Status**: Complete
**Date**: 2026-02-14

---

## Table of Contents

1. [Core Data Models (pandas)](#1-core-data-models-pandas)
   - [1.1 DataFrame Internal Structure](#11-dataframe-internal-structure)
   - [1.2 Series Internal Structure](#12-series-internal-structure)
   - [1.3 Block Model](#13-block-model)
   - [1.4 Index Hierarchy](#14-index-hierarchy)
   - [1.5 ExtensionArray Contract](#15-extensionarray-contract)
   - [1.6 DType Hierarchy](#16-dtype-hierarchy)
2. [State Transitions](#2-state-transitions)
   - [2.1 BlockManager Mutations](#21-blockmanager-mutations)
   - [2.2 Copy-on-Write (CoW) Semantics](#22-copy-on-write-cow-semantics)
   - [2.3 Index Mutability and Caching](#23-index-mutability-and-caching)
   - [2.4 Column DType Promotion](#24-column-dtype-promotion)
   - [2.5 Block Consolidation](#25-block-consolidation)
3. [Invariant Obligations](#3-invariant-obligations)
   - [3.1 Index-Length == Rows](#31-index-length--rows)
   - [3.2 Block Shape Alignment](#32-block-shape-alignment)
   - [3.3 DType Consistency Within Blocks](#33-dtype-consistency-within-blocks)
   - [3.4 Index Uniqueness Requirements](#34-index-uniqueness-requirements)
   - [3.5 NA Propagation Rules](#35-na-propagation-rules)
   - [3.6 Sort Stability Guarantees](#36-sort-stability-guarantees)
4. [FrankenPandas Comparison](#4-frankenpandas-comparison)
   - [4.1 Structural Mapping](#41-structural-mapping)
   - [4.2 Divergences from pandas Internals](#42-divergences-from-pandas-internals)
   - [4.3 Invariant Mapping Table](#43-invariant-mapping-table)

---

## 1. Core Data Models (pandas)

### 1.1 DataFrame Internal Structure

A `DataFrame` is a 2D labeled data structure backed by a `BlockManager` (defined in
`core/internals/managers.py`). The `BlockManager` holds:

| Component | Type | Description |
|-----------|------|-------------|
| `blocks` | `tuple[Block, ...]` | Ordered tuple of homogeneous-dtype data blocks |
| `axes` | `list[Index]` | Two-element list: `axes[0]` = column Index (items), `axes[1]` = row Index |
| `_blknos` | `ndarray[intp]` | Maps column position i to block number |
| `_blklocs` | `ndarray[intp]` | Maps column position i to within-block column position |
| `_known_consolidated` | `bool` | Whether consolidation state has been checked |
| `_is_consolidated` | `bool` | Whether same-dtype blocks have been merged |

**Key relationships:**

```
DataFrame
  |
  +-- _mgr: BlockManager (ndim=2)
        |
        +-- axes[0]: Index        # Column labels
        +-- axes[1]: Index        # Row index
        +-- blocks: tuple[Block]  # Data storage
        |     |
        |     +-- values: ndarray | ExtensionArray  # 2D for numpy, 1D for EA
        |     +-- _mgr_locs: BlockPlacement         # Which columns this block owns
        |     +-- refs: BlockValuesRefs              # CoW reference tracking
        |
        +-- _blknos[i] -> block number for column i
        +-- _blklocs[i] -> position within that block for column i
```

**Source**: `core/internals/managers.py:1069` (class `BlockManager(libinternals.BlockManager, BaseBlockManager)`)

The `BlockManager.__init__` verifies integrity by ensuring:
- All block `ndim` values equal 2.
- Block shapes `block.shape[1:]` match manager `shape[1:]`.
- Sum of block item counts equals `len(self.items)`.

### 1.2 Series Internal Structure

A `Series` is a 1D labeled structure backed by a `SingleBlockManager` (defined in
`core/internals/managers.py:1986`):

| Component | Type | Description |
|-----------|------|-------------|
| `blocks` | `tuple[Block]` | Always exactly one block |
| `axes` | `list[Index]` | Single-element list: `axes[0]` = the index |
| `is_single_block` | `True` (class const) | Always true |
| `ndim` | `1` (property) | Always 1 |

```
Series
  |
  +-- _mgr: SingleBlockManager (ndim=1)
        |
        +-- axes[0]: Index        # Row index
        +-- blocks[0]: Block      # Single data block
              |
              +-- values: ndarray | ExtensionArray  # 1D
              +-- _mgr_locs: BlockPlacement(slice(0, n))
              +-- refs: BlockValuesRefs
```

**Source**: `core/internals/managers.py:1986` (`class SingleBlockManager(BaseBlockManager)`)

The `SingleBlockManager` is always consolidated and contains exactly one block. It
exposes convenience properties:
- `_block` -> `self.blocks[0]` (cached)
- `array` -> `self.blocks[0].values`
- `index` -> `self.axes[0]`
- `dtype` -> `self._block.dtype`

### 1.3 Block Model

Blocks are the fundamental storage unit. Each block holds a contiguous array of
homogeneous dtype and tracks which columns in the BlockManager it owns.

**Source**: `core/internals/blocks.py:138` (`class Block(PandasObject, libinternals.Block)`)

#### Block Class Hierarchy

```
Block (base)
  |
  +-- NumpyBlock              # Holds np.ndarray values
  +-- ExtensionBlock          # Holds ExtensionArray values (1D-only EAs)
```

Note: As of pandas 2.x, the older specialized block types (`IntBlock`, `FloatBlock`,
`ObjectBlock`, `DatetimeLikeBlock`, etc.) have been consolidated. The distinction is
now between `NumpyBlock` (numpy-backed) and the generic `ExtensionBlock`.

#### Key Block Attributes

| Attribute | Type | Description |
|-----------|------|-------------|
| `values` | `np.ndarray \| ExtensionArray` | The backing data array |
| `ndim` | `int` | 1 for SingleBlockManager, 2 for BlockManager |
| `_mgr_locs` | `BlockPlacement` | Which manager columns this block holds |
| `refs` | `BlockValuesRefs` | Copy-on-Write reference tracker |
| `is_numeric` | `bool` | Whether block holds numeric data |
| `is_object` | `bool` | Whether dtype is object |
| `is_extension` | `bool` | Whether values are ExtensionArray |
| `_can_consolidate` | `bool` | True for numpy blocks, False for extension blocks |
| `_can_hold_na` | `bool` | True for float/object/extension, False for int/uint/bool |
| `fill_value` | scalar | NA sentinel for this dtype (np.nan, NaT, pd.NA, etc.) |

#### Block Operations

- `apply(func)` -- apply function to values, return list of new blocks
- `reduce(func)` -- column-wise reduction, returns single-row block
- `_split()` -- split multi-column block into single-column blocks (generator)
- `make_block(values, placement)` -- create new block with type inference
- `should_store(value)` -- whether a value can be stored without dtype cast
- `_can_hold_element(element)` -- whether a scalar can fit in this dtype

### 1.4 Index Hierarchy

All Index classes inherit from `Index` (defined in `core/indexes/base.py:313`).

```
Index (base, core/indexes/base.py)
  |
  +-- RangeIndex (core/indexes/range.py:78)
  |     Stores start/stop/step without materializing array.
  |     Memory-efficient for default integer indexes.
  |
  +-- MultiIndex (core/indexes/multi.py:197)
  |     Hierarchical index using codes + levels arrays.
  |     levels: FrozenList of Index objects (one per level)
  |     codes: FrozenList of ndarray[intp] (integer codes per level)
  |
  +-- DatetimeIndex (core/indexes/datetimes.py:142) -> DatetimeTimedeltaMixin
  |     Backed by DatetimeArray, supports tz-aware operations.
  |
  +-- TimedeltaIndex (core/indexes/timedeltas.py:75) -> DatetimeTimedeltaMixin
  |     Backed by TimedeltaArray for duration data.
  |
  +-- PeriodIndex (core/indexes/period.py:78) -> DatetimeIndexOpsMixin
  |     Backed by PeriodArray for regular time periods.
  |
  +-- CategoricalIndex (core/indexes/category.py:78) -> NDArrayBackedExtensionIndex
  |     Backed by Categorical array.
  |
  +-- IntervalIndex (core/indexes/interval.py:159) -> ExtensionIndex
        Backed by IntervalArray for interval/bin data.
```

**Key Index properties:**
- `_data`: The backing array (ndarray or ExtensionArray)
- `_engine`: Lazily-constructed hash-table engine for O(1) label lookup
- `_cache`: Dictionary for memoized computed properties
- `name`: Optional index name
- `dtype`: The dtype of the index data
- `is_monotonic_increasing` / `is_monotonic_decreasing`: cached sort order checks
- `is_unique`: Whether all labels are distinct (cached)
- `_no_setting_name`: Flag to prevent name mutation

The base `Index` class provides the `_engine` property (lazily built), which
creates a typed hash-table engine (e.g., `Int64Engine`, `ObjectEngine`) for
efficient `get_loc()` lookups. The engine type is selected from `_masked_engines`
dict based on dtype.

### 1.5 ExtensionArray Contract

**Source**: `core/arrays/base.py:112` (`class ExtensionArray`)

An ExtensionArray is a custom 1D array type that pandas recognizes natively. It
enables third-party dtypes (Arrow-backed, masked nullable integers, etc.).

#### Required Abstract Methods (Must Be Implemented)

| Method | Signature | Purpose |
|--------|-----------|---------|
| `_from_sequence` | `(cls, scalars, dtype=None)` | Construct from iterable of scalars |
| `_from_factorized` | `(cls, values, original)` | Reconstruct from factorized form |
| `__getitem__` | `(self, item)` | Positional indexing |
| `__len__` | `(self)` | Length |
| `__eq__` | `(self, other)` | Element-wise equality |
| `dtype` | property | Associated ExtensionDtype instance |
| `nbytes` | property | Memory consumption in bytes |
| `isna` | `(self)` | Boolean array of missing values |
| `take` | `(self, indices, allow_fill, fill_value)` | Positional take with fill |
| `copy` | `(self)` | Deep copy |
| `_concat_same_type` | `(cls, to_concat)` | Concatenation of same-type arrays |
| `interpolate` | `(self, ...)` | Interpolation |

#### Performance-Critical Optional Methods

| Method | Purpose |
|--------|---------|
| `fillna` | Fill missing values (avoids `astype(object)` round-trip) |
| `_pad_or_backfill` | Forward/backward fill |
| `dropna` | Drop missing values |
| `unique` | Unique values |
| `factorize` | Encode as integer codes + uniques |
| `_values_for_argsort` | Array suitable for sorting |
| `searchsorted` | Binary search for insertion points |
| `_accumulate` | Cumulative operations (cumsum, cummax, etc.) |
| `_reduce` | Aggregation operations (sum, mean, min, max, etc.) |
| `_hash_pandas_object` | Hashing for groupby/merge |

### 1.6 DType Hierarchy

**Source**: `core/dtypes/base.py:46` (`class ExtensionDtype`), `core/dtypes/dtypes.py`

```
numpy.dtype (built-in numpy dtypes)
  |
  +-- float16, float32, float64
  +-- int8, int16, int32, int64
  +-- uint8, uint16, uint32, uint64
  +-- bool_
  +-- object_
  +-- datetime64[ns], timedelta64[ns]
  +-- complex64, complex128
  +-- str_ (string)

ExtensionDtype (pandas extension dtypes base, core/dtypes/base.py)
  |
  +-- PandasExtensionDtype (core/dtypes/dtypes.py:114)
  |     |
  |     +-- CategoricalDtype         # categories + ordered flag
  |     +-- DatetimeTZDtype          # datetime64[ns, tz] with timezone
  |     +-- PeriodDtype              # period[freq]
  |     +-- IntervalDtype            # interval[subtype, closed]
  |     +-- SparseDtype              # Sparse[subtype, fill_value]
  |
  +-- BaseMaskedDtype               # Nullable integer/float/bool base
  |     +-- Int8Dtype, Int16Dtype, Int32Dtype, Int64Dtype
  |     +-- UInt8Dtype, ... UInt64Dtype
  |     +-- Float32Dtype, Float64Dtype
  |     +-- BooleanDtype
  |
  +-- StringDtype                   # pd.StringDtype (object or arrow backed)
  +-- ArrowDtype                    # General pyarrow-backed dtype
  +-- NumpyEADtype                  # Wrapper for numpy dtypes as EA dtypes
```

**Required ExtensionDtype interface:**

| Attribute/Method | Purpose |
|-----------------|---------|
| `type` (property) | The scalar type for this dtype |
| `name` (property) | String identifier (e.g., "Int64", "category") |
| `construct_array_type()` | Returns the associated ExtensionArray class |
| `_is_numeric` | Whether this dtype is numeric |
| `_is_boolean` | Whether this dtype is boolean |
| `_get_common_dtype(dtypes)` | Find common dtype for a list of dtypes |
| `na_value` | The NA sentinel for this dtype (default: `np.nan`) |
| `_metadata` | Tuple of attribute names for equality/hashing |

---

## 2. State Transitions

### 2.1 BlockManager Mutations

The BlockManager changes state in the following scenarios:

**Column insertion** (`BlockManager.insert`, `core/internals/managers.py:1515`):
1. Creates a new single-column block.
2. Appends it to `self.blocks`.
3. Updates `_blknos` and `_blklocs` arrays.
4. Increments `mgr_locs` of all blocks above the insertion point.
5. Sets `_known_consolidated = False`.
6. Emits `PerformanceWarning` if >100 non-extension blocks (fragmentation).

**Column setting** (`BlockManager.iset`, `core/internals/managers.py:1251`):
1. If inplace and dtype matches and block has no external references, sets in-place.
2. If block has CoW references, splits the block first (`_iset_split_block`).
3. If dtype does not match, removes old block, creates new block, appends.
4. Updates `_blknos` and `_blklocs` accordingly.

**Column deletion** (`BlockManager.idelete`, `core/internals/managers.py:1600`):
1. Creates a boolean mask of deleted positions.
2. Takes non-deleted columns via `_slice_take_blocks_ax0`.
3. Returns a new `BlockManager` (does not mutate in-place).

**Reindexing** (`BlockManager.reindex_indexer`, `core/internals/managers.py:792`):
1. If `indexer is None` and same axis object, returns shallow copy.
2. For axis=0, uses `_slice_take_blocks_ax0` to select/fill columns.
3. For axis=1, calls `blk.take_nd()` on each block to reindex rows.
4. Returns a new `BlockManager` with new axes.

### 2.2 Copy-on-Write (CoW) Semantics

**Source**: `BlockValuesRefs` in `pandas._libs.internals`, CoW logic throughout managers.py

Pandas implements Copy-on-Write via `BlockValuesRefs`, a reference-tracking object:

| Scenario | Copy? | Mechanism |
|----------|-------|-----------|
| `df[col]` (column access) | No -- returns view | `SingleBlockManager` shares `refs` with parent block |
| `df.copy(deep=False)` | No data copy | New BlockManager with shared `refs` |
| `df.copy(deep=True)` | Full copy | New blocks with new `refs`, consolidation triggered |
| `df[col] = new_values` (setitem) | Copy if refs exist | `_has_no_reference()` check; splits block if shared |
| `df.iloc[row, col] = val` | Copy column if refs | `column_setitem` copies column block if CoW active |
| Slicing `df[1:5]` | View | Blocks share refs with parent |
| Binary ops `df + df2` | Always new | New blocks allocated, no ref sharing |
| Inplace ops (where, putmask) | Copy if refs | Block is copied before mutation when refs exist |

**Reference tracking flow:**
1. `BlockValuesRefs` uses weak references (`referenced_blocks`) to track all blocks sharing the same underlying array.
2. `has_reference()` returns `True` if any other block still references this data.
3. When mutation is needed and `has_reference() == True`, the block is copied first.
4. `add_references(mgr)` links refs between two managers with identical block structure.

### 2.3 Index Mutability and Caching

Indexes in pandas are **immutable** by design. The `Index` object stores:

- `_data`: The underlying array (set once at construction)
- `_cache`: A dictionary for memoized computed properties
- `_engine`: Lazily-built hash engine (via `@cache_readonly`)

**Cached properties** (computed once, stored in `_cache`):
- `is_unique` -- whether all labels are distinct
- `is_monotonic_increasing` / `is_monotonic_decreasing`
- `_engine` -- the hash-table engine for O(1) lookups
- `inferred_type` -- inferred type string
- `hasnans` -- whether index contains NaN values

**Cache invalidation**: Index caches are never invalidated because indexes are
immutable. Any "modification" returns a new Index object (e.g., `index.insert()`,
`index.delete()`, `index.set_names()`).

**Name mutation exception**: `Index.name` can be set in-place (it does not affect
data), but `_no_setting_name` flag can prevent this.

### 2.4 Column DType Promotion

**Source**: `core/dtypes/cast.py:find_common_type`, `core/dtypes/cast.py:can_hold_element`

Promotion rules during operations:

| Operation | Input Types | Result Type |
|-----------|-------------|-------------|
| Arithmetic | int + float | float64 |
| Arithmetic | int + int | int64 (may promote to float64 if NaN introduced) |
| Arithmetic | bool + int | int64 |
| Arithmetic | bool + float | float64 |
| Division | any numeric | float64 (always) |
| Comparison | any + any | bool |
| Concatenation | int + float | float64 |
| Concatenation | int + object | object |
| Setting NaN | int column | float64 (pre-nullable) or stays Int64 (nullable) |
| Mixed types | numeric + string | object |
| Extension | Int64 + float | Float64 (nullable float) |

**Block splitting on type mismatch**: When `iset` encounters a value whose dtype
does not match the existing block's dtype (`blk.should_store(value)` returns
`False`), the block is split and a new block of the appropriate type is created.

### 2.5 Block Consolidation

**Source**: `core/internals/managers.py:1928-1951`

Consolidation merges multiple blocks of the same dtype into a single block. This is
an optimization that improves memory locality and reduces iteration overhead.

**When consolidation is checked:**
- `is_consolidated()` -- checks if any two blocks share the same `_consolidate_key` (dtype name)
- `_consolidate_check()` -- sets `_is_consolidated` flag

**When consolidation happens:**
- `_consolidate_inplace()` -- called after certain mutations (deep copy, replace_list)
- Calls `_consolidate(self.blocks)` which groups blocks by dtype and merges them
- Only non-extension blocks can consolidate (`_can_consolidate` is `False` for extension blocks)
- DatetimeTZDtype blocks explicitly do not consolidate despite being 2D-capable

**Consolidation trigger points:**
1. `BlockManager.copy(deep=True)` -- consolidates after copy
2. `replace_list` -- consolidates after multi-value replacement
3. Manual `DataFrame._consolidate()` -- explicit user-triggered consolidation

**Fragmentation warning**: When >100 non-extension blocks exist (typically from
repeated `frame.insert`), a `PerformanceWarning` is raised suggesting
`pd.concat(axis=1)` instead.

---

## 3. Invariant Obligations

### 3.1 Index-Length == Rows

**Invariant**: `len(axes[1]) == block.shape[1]` for all blocks (i.e., the row index
length must match all block row counts).

**Enforcement**:
- `BlockManager._verify_integrity()` (`managers.py:1101`): Checks that
  `block.shape[1:] == mgr_shape[1:]` for every block.
- `BaseBlockManager._validate_set_axis()` (`managers.py:277`): When setting a new
  axis, raises `ValueError` if `new_len != old_len` (except for empty DataFrames).
- `SingleBlockManager.from_array()`: Block placement is `slice(0, len(index))`,
  tying block size to index length.

**Violation consequence**: `AssertionError` or `ValueError` during construction or
axis replacement.

### 3.2 Block Shape Alignment

**Invariant**: For every block in a `BlockManager`:
- `block.shape[1:] == manager.shape[1:]` (row dimensions match)
- `sum(len(block.mgr_locs) for block in blocks) == len(items)` (total column coverage)
- Each column position 0..n-1 is owned by exactly one block

**Enforcement**:
- `BlockManager._verify_integrity()` checks both conditions.
- `_blknos` / `_blklocs` arrays are rebuilt after mutations to ensure correct mapping.
- `_rebuild_blknos_and_blklocs()` reconstructs the mapping from block placements.

### 3.3 DType Consistency Within Blocks

**Invariant**: All elements within a single block have the same dtype. A block's
`values` array is a homogeneous numpy ndarray or ExtensionArray.

**Enforcement**:
- Blocks are constructed via `new_block()` which calls `maybe_coerce_values()` to
  ensure the values array is properly typed.
- `Block.should_store(value)` returns `True` only when `value.dtype == self.dtype`.
- When a value of different dtype is assigned, the block is split and a new block
  of the appropriate dtype is created.
- Extension blocks are never consolidated with numpy blocks (different
  `_consolidate_key`).

### 3.4 Index Uniqueness Requirements

**Invariant**: Index uniqueness is NOT globally required but IS required for certain
operations.

| Operation | Uniqueness Required? | Behavior When Violated |
|-----------|---------------------|----------------------|
| Construction | No | Duplicates are allowed |
| `df.loc[label]` | No | Returns DataFrame/Series for duplicates |
| `df.reindex(new_index)` | Yes (on source) | `InvalidIndexError` if source has duplicates |
| `df.join()` | Depends on `validate` | Can raise if specified |
| `df.merge()` | No | Produces cross-product for duplicates |
| `index.is_unique` | N/A | Cached boolean check |
| Set operations (`union`, `intersection`) | No | Deduplication happens implicitly |

**`_validate_can_reindex(indexer)`**: Called during reindex to ensure the source index
supports the reindex (raises for duplicates when indexer has -1 fill values).

**`DuplicateLabelError`**: Raised when `allows_duplicate_labels=False` is set on the
DataFrame/Series and a duplicate-producing operation is attempted.

### 3.5 NA Propagation Rules

**Per-dtype NA semantics:**

| DType Family | NA Value | Propagation Behavior |
|-------------|----------|---------------------|
| `float64` | `np.nan` | NaN propagates through arithmetic (IEEE 754) |
| `int64` (numpy) | Cannot hold NA | Column promoted to `float64` when NA introduced |
| `Int64` (nullable) | `pd.NA` | NA propagates, result stays nullable integer |
| `bool` (numpy) | Cannot hold NA | Promoted to `object` or `boolean` |
| `boolean` (nullable) | `pd.NA` | NA propagates per Kleene logic |
| `object` | `np.nan` or `None` | NaN/None treated as missing |
| `datetime64` | `NaT` | NaT propagates through datetime ops |
| `timedelta64` | `NaT` | NaT propagates through timedelta ops |
| `string` (StringDtype) | `pd.NA` | NA propagates |
| `category` | `np.nan` | NaN allowed if categories permit |
| Arrow-backed | `pd.NA` | NA propagates per Arrow semantics |

**Key rules:**
- `isna()` / `notna()` are the canonical NA detection functions.
- `na_value_for_dtype(dtype)` returns the appropriate NA sentinel per dtype.
- Aggregations (sum, mean, etc.) skip NA by default (`skipna=True`).
- Comparison with NA: `x == np.nan` is `False`; use `isna(x)` instead.
- Nullable dtypes use three-valued logic: `True & NA == NA`, `False & NA == False`.

### 3.6 Sort Stability Guarantees

**Invariant**: pandas guarantees stable sorting in all sort operations.

| Operation | Sort Algorithm | Stability |
|-----------|---------------|-----------|
| `DataFrame.sort_values()` | mergesort (default) | Stable |
| `DataFrame.sort_index()` | mergesort (default) | Stable |
| `Index.sort_values()` | mergesort | Stable |
| `GroupBy` | Preserves group order | Stable within groups |
| `merge()` sort | mergesort | Stable |
| `nargsort()` | mergesort (internal) | Stable |

**Source**: `core/sorting.py:nargsort`, which uses `np.argsort(kind="mergesort")`.

Stability means: when two elements compare equal, their relative order in the input
is preserved in the output. This is critical for:
- GroupBy group ordering (groups appear in order of first occurrence)
- Multi-key sorting (secondary key order preserved within primary key ties)
- Deterministic output for reproducible analyses

---

## 4. FrankenPandas Comparison

### 4.1 Structural Mapping

| pandas Concept | FrankenPandas Equivalent | Crate | Notes |
|---------------|------------------------|-------|-------|
| **DataFrame** | `DataFrame` struct | `fp-frame` | `{ index: Index, columns: BTreeMap<String, Column> }` |
| **Series** | `Series` struct | `fp-frame` | `{ name: String, index: Index, column: Column }` |
| **BlockManager** | *No equivalent* | -- | FP uses per-column storage, not block-based |
| **SingleBlockManager** | *No equivalent* | -- | Series directly owns its Column |
| **Block** | `Column` struct | `fp-columnar` | `{ dtype: DType, values: Vec<Scalar>, validity: ValidityMask }` |
| **Index** | `Index` struct | `fp-index` | `{ labels: Vec<IndexLabel>, duplicate_cache: OnceCell<bool>, sort_order_cache: OnceCell<SortOrder> }` |
| **RangeIndex** | `Index::from_range()` | `fp-index` | Materializes to `Vec<IndexLabel>` (no lazy range) |
| **MultiIndex** | *Not implemented* | -- | Only flat indexes supported |
| **DatetimeIndex** | *Not implemented* | -- | No datetime-specific index type |
| **ExtensionArray** | `ColumnData` enum | `fp-columnar` | `Float64(Vec<f64>) \| Int64(Vec<i64>) \| Bool(Vec<bool>) \| Utf8(Vec<String>)` |
| **ValidityMask** | `ValidityMask` struct | `fp-columnar` | Packed `Vec<u64>` bitvec (64 bits per word) |
| **ExtensionDtype** | `DType` enum | `fp-types` | `{ Null, Bool, Int64, Float64, Utf8 }` |
| **Scalar** | `Scalar` enum | `fp-types` | `{ Null(NullKind), Bool(bool), Int64(i64), Float64(f64), Utf8(String) }` |
| **NullKind** | `NullKind` enum | `fp-types` | `{ Null, NaN, NaT }` -- distinguishes NA flavors |
| **BlockPlacement** | *No equivalent* | -- | Each column is independent |
| **BlockValuesRefs (CoW)** | *No equivalent* | -- | Rust ownership model, no CoW needed |
| **IndexLabel** | `IndexLabel` enum | `fp-index` | `Int64(i64) \| Utf8(String)` |
| **AlignmentPlan** | `AlignmentPlan` struct | `fp-index` | `{ union_index, left_positions, right_positions }` |
| **JoinType** | `JoinType` enum | `fp-join` | `Inner \| Left \| Right \| Outer` |
| **GroupBy** | `groupby_sum` etc. | `fp-groupby` | Free functions with arena allocation |
| **Expr tree** | `Expr` enum | `fp-expr` | `Series \| Add \| Literal` with IVM delta propagation |
| **RuntimePolicy** | `RuntimePolicy` | `fp-runtime` | `Strict \| Hardened \| Permissive` -- no pandas equivalent |
| **EvidenceLedger** | `EvidenceLedger` | `fp-runtime` | Decision audit trail -- no pandas equivalent |

### 4.2 Divergences from pandas Internals

#### D1: No Block Manager -- Per-Column Storage

**pandas**: DataFrame stores data in a `BlockManager` containing multiple `Block`
objects, each holding columns of the same dtype in a 2D numpy array. Column i is
located via `_blknos[i]` and `_blklocs[i]` indirection.

**FrankenPandas**: DataFrame stores `BTreeMap<String, Column>` -- each column is an
independent `Column` struct with its own `Vec<Scalar>`, `ValidityMask`, and `DType`.
There is no block concept, no consolidation, and no cross-column data sharing.

**Implications**:
- No consolidation overhead or fragmentation warnings.
- No cross-column SIMD optimizations from shared contiguous memory.
- Simpler mutation model (no block splitting).
- Column access is O(log n) via BTreeMap vs O(1) via ndarray indexing.

#### D2: No Copy-on-Write -- Rust Ownership

**pandas**: CoW uses `BlockValuesRefs` with weak references to track shared data
and trigger lazy copies on mutation.

**FrankenPandas**: Rust's ownership and borrowing system makes CoW unnecessary. Data
is either owned (moved), borrowed (immutable reference), or explicitly cloned.
Operations return new `Series`/`DataFrame` values; there are no views or shared
mutable state.

**Implications**:
- No reference-counting overhead.
- No subtle view vs copy bugs.
- Higher memory use for operations that would be views in pandas (e.g., column access
  always clones in the current implementation).

#### D3: Scalar-Level Typing vs Array-Level Typing

**pandas**: Each column/block has a single dtype. Values within are homogeneous numpy
arrays or ExtensionArrays. Type promotion happens at the array level via
`find_common_type`.

**FrankenPandas**: `Column` stores `Vec<Scalar>` where each `Scalar` is a tagged
enum. DType is inferred from the collection via `infer_dtype()`. The `ColumnData`
enum provides a typed-array fast path for vectorized operations, but the canonical
storage remains `Vec<Scalar>`.

**Implications**:
- Each scalar carries its own type tag (1-byte enum discriminant overhead per element).
- Mixed-type columns are impossible by design (dtype is enforced at Column construction).
- Type coercion happens per-scalar via `cast_scalar_owned`.
- Vectorized path (`ColumnData::Float64(Vec<f64>)`) materializes from scalars on demand.

#### D4: Flat Index Only -- No Hierarchy

**pandas**: Full Index hierarchy including `MultiIndex` (codes + levels),
`DatetimeIndex`, `PeriodIndex`, `CategoricalIndex`, `IntervalIndex`.

**FrankenPandas**: Single `Index` struct with `Vec<IndexLabel>` where `IndexLabel`
is `Int64(i64) | Utf8(String)`. No multi-level, datetime, or categorical indexes.

**Implications**:
- No hierarchical groupby/pivot without flattening.
- No datetime-aware range operations (no freq-based slicing).
- No memory optimization for monotonic integer ranges (always materialized).
- Adaptive backend: sorted indexes get binary search, unsorted get HashMap fallback.

#### D5: Explicit NA Discrimination

**pandas**: NA handling is dtype-dependent. `np.nan` for float, `NaT` for datetime,
`pd.NA` for nullable extension types. `np.nan` cannot be stored in integer arrays
(forces float promotion).

**FrankenPandas**: `NullKind` enum explicitly distinguishes `Null`, `NaN`, and `NaT`.
`Scalar::Null(NullKind::NaN)` and `Scalar::Float64(f64::NAN)` are both treated as
missing by `is_missing()`, but they preserve the distinction for round-tripping.
Integer columns CAN hold `Scalar::Null(NullKind::Null)` without promotion.

**Implications**:
- No forced int->float promotion when NA is introduced (matches pandas nullable int semantics).
- NA semantics are consistent across all dtypes.
- NaN-vs-Null distinction is preserved for debugging and compatibility verification.

#### D6: Runtime Policy Layer

**pandas**: No equivalent. Operations always proceed (with warnings for some edge cases).

**FrankenPandas**: `RuntimePolicy` (Strict/Hardened/Permissive) gates operations:
- **Strict**: Rejects duplicate index alignment, raises on any unknown feature.
- **Hardened**: Allows operations with size bounds (e.g., max join output rows).
- **Permissive**: Best-effort, like pandas.

The `EvidenceLedger` records all policy decisions for audit. This is the AACE
(Alignment-Aware Columnar Execution) layer.

#### D7: BTreeMap Column Ordering vs Positional

**pandas**: Columns maintain insertion order. Positional access via integer index is O(1).

**FrankenPandas**: `BTreeMap<String, Column>` sorts columns alphabetically by name.
No positional access by integer. Column order is deterministic but not insertion-ordered.

**Implications**:
- `column_names()` returns alphabetically sorted list.
- No iloc-style positional column access.
- `from_dict` accepts `column_order` parameter but BTreeMap still sorts internally.

#### D8: Immutable-by-Default Operations

**pandas**: Many operations support `inplace=True` parameter for in-place mutation.
The `BlockManager` supports both in-place and copy-on-write mutation paths.

**FrankenPandas**: All operations return new values. `with_column`, `drop_column`,
`rename_columns`, `filter_rows`, etc. all return new `DataFrame` instances. The Rust
ownership model makes this natural -- old data is dropped when no longer referenced.

### 4.3 Invariant Mapping Table

| pandas Invariant | FrankenPandas Equivalent | How Enforced |
|-----------------|------------------------|--------------|
| `len(index) == nrows` for all blocks | `index.len() == column.len()` for all columns | `DataFrame::new()` and `Series::new()` check at construction; `FrameError::LengthMismatch` on violation |
| Block shapes match manager axes | Each column independently sized | `Column.len()` checked against `Index.len()` per-column |
| DType homogeneity within block | DType homogeneity within Column | `Column::new(dtype, values)` coerces all values to target dtype via `cast_scalar_owned` |
| Index uniqueness for reindex | Duplicate detection via `has_duplicates()` with `OnceCell` memoization | Strict mode rejects duplicates for alignment; other modes use first-match semantics |
| NA propagation per dtype | Unified NA propagation via `Scalar::is_missing()` | `ValidityMask.and_mask()` for binary ops; `NullKind` preserves NaN vs Null distinction |
| Sort stability (mergesort) | Rust `sort_by` (Tim sort, stable) | `Index::argsort()` uses `sort_by` which is stable in Rust std library |
| CoW prevents unintended mutation | Rust ownership prevents aliased mutation | No runtime overhead; compile-time guarantee via borrow checker |
| Consolidation reduces fragmentation | No consolidation needed | Per-column storage has no fragmentation concept |
| Block `_can_hold_na` determines NA storage | All dtypes can hold NA via `Scalar::Null(_)` | No int-to-float promotion; nullable by default |
| `BlockValuesRefs` tracks sharing | Rust `Clone` / ownership | Explicit `.clone()` for copies; no implicit sharing |

---

## Appendix A: Source File Reference

### pandas Legacy Code

| File | Key Classes/Functions |
|------|----------------------|
| `core/internals/managers.py` | `BaseBlockManager`, `BlockManager`, `SingleBlockManager` |
| `core/internals/blocks.py` | `Block`, `NumpyBlock`, `new_block`, `ensure_block_shape` |
| `core/frame.py` | `DataFrame` |
| `core/series.py` | `Series` |
| `core/indexes/base.py` | `Index` (base class) |
| `core/indexes/range.py` | `RangeIndex` |
| `core/indexes/multi.py` | `MultiIndex` |
| `core/indexes/datetimes.py` | `DatetimeIndex` |
| `core/indexes/timedeltas.py` | `TimedeltaIndex` |
| `core/indexes/period.py` | `PeriodIndex` |
| `core/indexes/category.py` | `CategoricalIndex` |
| `core/indexes/interval.py` | `IntervalIndex` |
| `core/arrays/base.py` | `ExtensionArray` (ABC) |
| `core/dtypes/base.py` | `ExtensionDtype` (ABC) |
| `core/dtypes/dtypes.py` | `CategoricalDtype`, `DatetimeTZDtype`, `PeriodDtype`, etc. |
| `core/dtypes/cast.py` | `find_common_type`, `can_hold_element`, type promotion |
| `core/dtypes/missing.py` | `isna`, `notna`, `na_value_for_dtype` |

### FrankenPandas Crates

| Crate | File | Key Types |
|-------|------|-----------|
| `fp-types` | `crates/fp-types/src/lib.rs` | `DType`, `Scalar`, `NullKind`, `TypeError`, `common_dtype`, `infer_dtype`, `cast_scalar_owned`, nanops |
| `fp-columnar` | `crates/fp-columnar/src/lib.rs` | `Column`, `ValidityMask`, `ColumnData`, `ArithmeticOp`, `ComparisonOp`, vectorized binary ops |
| `fp-index` | `crates/fp-index/src/lib.rs` | `Index`, `IndexLabel`, `AlignmentPlan`, `AlignMode`, `SortOrder`, `DuplicateKeep`, `align`, `align_union`, `align_inner` |
| `fp-frame` | `crates/fp-frame/src/lib.rs` | `Series`, `DataFrame`, `FrameError`, `concat_series`, `concat_dataframes` |
| `fp-join` | `crates/fp-join/src/lib.rs` | `JoinType`, `JoinedSeries`, `join_series` (Inner/Left/Right/Outer with arena) |
| `fp-groupby` | `crates/fp-groupby/src/lib.rs` | `groupby_sum`, `GroupByOptions`, `GroupByExecutionOptions` (arena allocation) |
| `fp-expr` | `crates/fp-expr/src/lib.rs` | `Expr`, `EvalContext`, `evaluate`, `Delta` (IVM) |
| `fp-runtime` | `crates/fp-runtime/src/lib.rs` | `RuntimePolicy`, `RuntimeMode`, `EvidenceLedger`, `DecisionAction` |
| `fp-io` | `crates/fp-io/src/lib.rs` | CSV/JSON I/O |
| `fp-conformance` | `crates/fp-conformance/src/lib.rs` | Conformance test harness |

---

## Appendix B: DType Promotion Matrix (FrankenPandas)

From `fp-types/src/lib.rs:common_dtype`:

```
            Null    Bool    Int64   Float64   Utf8
Null        Null    Bool    Int64   Float64   Utf8
Bool        Bool    Bool    Int64   Float64   ERROR
Int64       Int64   Int64   Int64   Float64   ERROR
Float64     Float64 Float64 Float64 Float64   ERROR
Utf8        Utf8    ERROR   ERROR   ERROR     Utf8
```

Division always promotes to Float64 regardless of input types. Bool is treated as
Int64 for arithmetic purposes (true=1, false=0).

---

*Generated for FrankenPandas Phase-2C, bead bd-2gi.23.4*
