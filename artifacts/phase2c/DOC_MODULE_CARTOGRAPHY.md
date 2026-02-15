# DOC-PASS-01: Full Module/Package Cartography with Ownership and Boundaries

**Bead:** bd-2gi.23.2
**Date:** 2026-02-14
**Source Tree:** `legacy_pandas_code/pandas/pandas/`

---

## Executive Summary

The pandas source tree (excluding tests) comprises **293 Python modules** and **40 Cython modules** across **51 packages**, totaling approximately **249,270 lines of Python** and **45,723 lines of Cython** (pyx only). An additional **~9,840 lines of C** exist in vendored parsers and datetime utilities, plus **~3,251 lines** in Cython template files (`.pxi.in`).

### LOC Distribution by Top-Level Package

| Package | Python LOC | Cython LOC (pyx) | Total | % of Total |
|---------|-----------|-------------------|-------|------------|
| `core/` | 179,016 | -- | 179,016 | 60.7% |
| `_libs/` | 116 | 45,723 | 45,839 | 15.5% |
| `io/` | 47,386 | -- | 47,386 | 16.1% |
| `plotting/` | 9,470 | -- | 9,470 | 3.2% |
| `_testing/` | 2,813 | -- | 2,813 | 1.0% |
| `conftest.py` | 2,160 | -- | 2,160 | 0.7% |
| `util/` | 2,132 | -- | 2,132 | 0.7% |
| `tseries/` | 1,426 | -- | 1,426 | 0.5% |
| `_config/` | 1,263 | -- | 1,263 | 0.4% |
| `errors/` | 1,154 | -- | 1,154 | 0.4% |
| `compat/` | 1,000 | -- | 1,000 | 0.3% |
| `_typing.py` | 578 | -- | 578 | 0.2% |
| `api/` | 375 | -- | 375 | 0.1% |
| `__init__.py` | 344 | -- | 344 | 0.1% |
| `arrays/` | 37 | -- | 37 | <0.1% |
| `testing.py` | 17 | -- | 17 | <0.1% |
| **TOTAL** | **249,270** | **45,723** | **294,993** | **100%** |

---

## Layer Architecture

The pandas codebase follows a layered architecture (bottom to top):

```
Layer 5: api/           Public API surface re-exports
Layer 4: plotting/      Visualization (depends on core, io.formats)
Layer 3: io/            I/O readers/writers (depends on core, _libs)
Layer 2: core/          Engine: DataFrame, Series, indexes, groupby, etc.
Layer 1: _libs/         Cython/C extensions: algos, parsers, datetime
Layer 0: _config/       Configuration system
         errors/        Exception hierarchy
         compat/        Compatibility shims
         _typing.py     Type aliases
         util/          Decorators, validators, helpers
```

**Layering constraints:**
- Layer 0 packages SHOULD NOT import from layers above (but `errors/` imports `_config` for config options, and `compat/` imports `_libs`)
- `_libs/` SHOULD NOT import from `core/` (VIOLATED -- see boundary violations)
- `core/` SHOULD NOT import from `io/` (VIOLATED -- see boundary violations)
- `io/` depends on `core/` (legitimate)
- `plotting/` depends on `core/` and `io.formats` (legitimate)
- `api/` re-exports from all layers (by design)

---

## Package-Level Dependency Map

```
                    +--------+
                    |  api/  |  <-- re-export surface
                    +--------+
                        |
            +-----------+-----------+
            |           |           |
       +--------+  +--------+  +----------+
       |plotting|  |  io/   |  |_testing/ |
       +--------+  +--------+  +----------+
            |       /   |           |
            +------+    |           |
            |           |           |
       +--------+       |           |
       | core/  |<------+-----------|
       +--------+
          |   |
    +-----+   +-----+
    |               |
+--------+    +--------+
| _libs/ |    |tseries/|
+--------+    +--------+
    |
+--------+    +--------+    +--------+    +--------+
|_config/|    |errors/ |    |compat/ |    | util/  |
+--------+    +--------+    +--------+    +--------+
```

### Import Count Matrix (from -> to)

| From \ To | core | _libs | util | errors | compat | _config | io | tseries | plotting | api |
|-----------|------|-------|------|--------|--------|---------|----|---------|----------|-----|
| **core/** | 879 | 192 | 121 | 55 | 51 | 31 | 22 | 8 | 1 | 1 |
| **io/** | 92 | 41 | 49 | 18 | 28 | 14 | 95 | 1 | -- | 1 |
| **plotting/** | 22 | 7 | 9 | 1 | -- | 1 | 5 | 1 | 28 | -- |
| **tseries/** | 4 | 11 | 1 | 1 | -- | -- | -- | 3 | -- | -- |
| **_testing/** | 10 | 4 | 1 | 2 | 4 | 2 | 2 | 1 | -- | -- |
| **compat/** | 2 | 3 | 5 | 1 | 3 | -- | -- | -- | -- | -- |
| **_config/** | -- | -- | 1 | -- | -- | 7 | -- | -- | -- | -- |
| **errors/** | -- | 1 | 1 | -- | -- | 1 | -- | -- | -- | -- |
| **api/** | 19 | 5 | -- | -- | -- | -- | 3 | -- | -- | 1 |

---

## Boundary Violations

### VIOLATION 1: `_libs/` imports from `core/` (upward dependency)

The Cython extension layer reaches up into the Python core layer, creating a circular dependency risk:

| Source | Target | Purpose |
|--------|--------|---------|
| `_libs/internals.pyx` | `core.internals.blocks`, `core.construction`, `core.internals.api` | Block construction in Cython internals manager |
| `_libs/lib.pyx` | `core.dtypes.missing`, `core.dtypes.generic`, `core.dtypes.cast`, `core.arrays.string_`, `core.arrays.BooleanArray`, `core.arrays.IntegerArray`, `core.arrays.FloatingArray` | Type inference and array construction |
| `_libs/parsers.pyx` | `core.arrays`, `core.dtypes.dtypes`, `core.dtypes.inference`, `core.arrays.boolean` | Parser needs to construct typed arrays |
| `_libs/testing.pyx` | `core.dtypes.missing` | Array comparison utility |
| `_libs/tslibs/offsets.pyx` | `core.dtypes.cast` | Scalar unboxing |

**Severity:** Moderate. These are mostly deferred (inside-function) imports to break import cycles, but they create a tangled dependency graph that complicates isolated testing and Rust port planning.

### VIOLATION 2: `core/` imports from `io/` (upward dependency)

The core engine layer reaches into the I/O layer, primarily for formatting and serialization:

| Source | Target | Purpose |
|--------|--------|---------|
| `core/frame.py` | `io.common`, `io.formats.*`, `io.formats.info`, `io.formats.style`, `io.stata`, `io.feather_format`, `io.parquet`, `io.orc`, `io.formats.xml` | DataFrame serialization methods (`.to_csv()`, `.to_parquet()`, etc.) |
| `core/arrays/*.py` | `io.formats.printing`, `io.formats.format`, `io.formats.console`, `io._util` | Array repr formatting |
| `core/indexes/*.py` | `io.formats.printing`, `io.formats.format` | Index repr formatting |
| `core/computation/*.py` | `io.formats.printing` | Expression repr formatting |
| `core/dtypes/cast.py` | `io._util._arrow_dtype_mapping` | Arrow dtype lookup |
| `core/config_init.py` | `io.formats.printing` | Formatter registration |
| `core/api.py` | `io.formats.format.set_eng_float_format` | API re-export |

**Severity:** Low-moderate. Most are deferred imports for formatting/repr. The `io.formats.printing` module is arguably misplaced -- it is a utility used by core and should live in `core/` or a shared utility layer.

### VIOLATION 3: `core/dtypes/cast.py` imports from `io._util`

This is a single import (`_arrow_dtype_mapping`) that leaks I/O-layer concerns into the dtype casting subsystem.

**Severity:** Low but architecturally concerning for the Rust port.

---

## Detailed Module Cartography

### 1. Root Level (`pandas/`)

| Module | LOC | Responsibility | Layer |
|--------|-----|---------------|-------|
| `__init__.py` | 344 | Package initialization; public namespace assembly; version check | api |
| `_typing.py` | 578 | Type aliases used across the codebase (Axis, Dtype, FilePath, etc.) | foundation |
| `conftest.py` | 2,160 | Pytest fixtures and configuration for the test suite | testing |
| `testing.py` | 17 | Thin shim that re-exports `pandas._testing` | api |

---

### 2. `_config/` -- Configuration System (1,263 LOC)

**Layer:** 0 (foundation)
**Depends on:** `_typing`, `util`
**Depended on by:** `core`, `io`, `plotting`, `_testing`, `errors`

| Module | LOC | Responsibility |
|--------|-----|---------------|
| `__init__.py` | 45 | Re-exports config registration and retrieval functions |
| `config.py` | 954 | Core option registration/retrieval engine (`register_option`, `get_option`, `set_option`, `option_context`). Maintains a global registry of configuration options with validators, defaults, and deprecation handling. |
| `dates.py` | 26 | Date-related display configuration constants |
| `display.py` | 62 | Display-specific configuration helpers (terminal detection, Unicode support) |
| `localization.py` | 176 | Locale detection and setting for number/date formatting |

---

### 3. `errors/` -- Exception Hierarchy (1,154 LOC)

**Layer:** 0 (foundation)
**Depends on:** `_config`, `_libs`, `util`
**Depended on by:** `core`, `io`, `plotting`, `_testing`, `compat`, `tseries`

| Module | LOC | Responsibility |
|--------|-----|---------------|
| `__init__.py` | 1,111 | Defines ALL pandas exception/warning classes: `PerformanceWarning`, `UnsortedIndexError`, `ParserError`, `MergeError`, `OptionError`, `OutOfBoundsDatetime`, `InvalidIndexError`, `AbstractMethodError`, `IntCastingNaNError`, `DataError`, etc. Single authoritative location for the full exception taxonomy. |
| `cow.py` | 43 | Copy-on-Write (CoW) specific warning class and helpers |

---

### 4. `compat/` -- Compatibility Shims (1,000 LOC)

**Layer:** 0 (foundation)
**Depends on:** `_libs`, `util`, `errors`
**Depended on by:** `core`, `io`, `plotting`, `_testing`

| Module | LOC | Responsibility |
|--------|-----|---------------|
| `__init__.py` | 173 | Platform detection (Windows, macOS, ARM, 32/64-bit), Python version checks, HDF5/zstd availability |
| `_constants.py` | 35 | Platform-specific constants (IS64, PY312, REF_COUNT) |
| `_optional.py` | 191 | Lazy optional dependency importer (`import_optional_dependency`). Central gate for all optional deps (numba, pyarrow, openpyxl, etc.) with version checking. |
| `numpy/__init__.py` | 48 | NumPy version-specific compatibility and function mapping |
| `numpy/function.py` | 376 | Argument validation for NumPy-compatible function signatures. Ensures pandas methods accept the same kwargs as numpy equivalents. |
| `pickle_compat.py` | 143 | Backward-compatible unpickling of old pandas objects |
| `pyarrow.py` | 34 | PyArrow version detection and availability checking |

---

### 5. `util/` -- Utilities (2,132 LOC)

**Layer:** 0 (foundation)
**Depends on:** minimal (some self-references)
**Depended on by:** ALL packages

| Module | LOC | Responsibility |
|--------|-----|---------------|
| `__init__.py` | 29 | Re-exports hash_array, hash_pandas_object |
| `_decorators.py` | 484 | Docstring substitution (`@Substitution`, `@Appender`), deprecation decorators, `@doc` for templated docstrings |
| `_doctools.py` | 206 | TablePlotter helper for generating RST/Sphinx documentation tables from DataFrames |
| `_exceptions.py` | 106 | Exception formatting utilities (`find_stack_level`, `rewrite_exception`) |
| `_print_versions.py` | 158 | `pd.show_versions()` implementation: collects Python, OS, and dependency version info |
| `_test_decorators.py` | 152 | Test-time decorators: `@skip_if_no`, `@skip_if_not_us_locale`, etc. |
| `_tester.py` | 60 | `pandas.test()` entry point for running the test suite via pytest |
| `_validators.py` | 482 | Argument validation helpers: `validate_bool_kwarg`, `validate_axis`, `validate_percentile`, `validate_ascending`, etc. |
| `version/__init__.py` | 455 | PEP 440 version parsing (vendored from `packaging.version`) |

---

### 6. `_libs/` -- Cython/C Extension Layer (45,839 LOC total)

**Layer:** 1 (performance foundation)
**Depends on:** NumPy, CPython C-API; reaches up to `core` in specific cases (VIOLATION)
**Depended on by:** `core`, `io`, `plotting`, `tseries`, `_testing`, `compat`

#### 6.1 `_libs/` Top-Level Cython Modules

| Module | LOC (pyx) | LOC (pyi) | LOC (pxd) | Responsibility |
|--------|-----------|-----------|-----------|---------------|
| `__init__.py` | 27 (py) | -- | -- | Package init, exposes `NaT`, `NaTType`, `iNaT`, `Timedelta`, `Timestamp`, `OutOfBoundsDatetime` |
| `algos.pyx` | 1,445 | 443 | 22 | Core algorithmic primitives: `kth_smallest`, `nancorr`, `nancov`, `is_monotonic`, `pad`/`backfill` (ffill/bfill), `rank_1d`, `take_2d` dispatch, `groupsort_indexer` |
| `arrays.pyx` | 198 | 40 | 11 | `NDArrayBacked` base class for arrays backed by a numpy ndarray; provides __getitem__/__setitem__ optimizations |
| `byteswap.pyx` | 85 | 5 | -- | Byte-swapping routines for reading binary file formats (SAS, Stata) |
| `groupby.pyx` | 2,325 | 234 | -- | Cython groupby aggregation kernels: `group_sum`, `group_mean`, `group_var`, `group_nth`, `group_last`, `group_rank`, `group_cumsum`, `group_shift`, etc. Performance-critical inner loops. |
| `hashing.pyx` | 200 | 9 | -- | Fast hashing: `hash_object_array` using SipHash for deterministic hashing of Python objects |
| `hashtable.pyx` | 128 | 274 | 189 | Hash table implementations (templated via `.pxi.in`): `Int64HashTable`, `Float64HashTable`, `StringHashTable`, `PyObjectHashTable`. Used by `factorize`, `unique`, `duplicated`. |
| `index.pyx` | 1,325 | 107 | -- | Index engines: `IndexEngine`, `DatetimeEngine`, `TimedeltaEngine`, `PeriodEngine`, `MaskedIndexEngine`. Binary search and hash-based lookup for index operations. |
| `indexing.pyx` | 28 | 17 | -- | `NDFrameIndexerBase` base class for `.loc`/`.iloc` accessor objects |
| `internals.pyx` | 1,026 | 96 | -- | Cython-optimized BlockManager operations: `BlockPlacement`, `SharedBlock`, `NumpyBlock`, `Block`. Fast block consolidation and iteration. |
| `interval.pyx` | 684 | 174 | -- | `Interval`, `IntervalMixin`, `IntervalTree` implementation. Optimized interval containment and overlap checks. |
| `join.pyx` | 880 | 79 | -- | Join/merge algorithms: `left_join_indexer`, `inner_join_indexer`, `outer_join_indexer`, `asof_join_backward`, `asof_join_nearest`. Templated for multiple dtypes. |
| `lib.pyx` | 3,325 | 238 | 6 | Utility megamodule: `maybe_convert_objects`, `infer_dtype`, `is_float`, `is_integer`, `no_default` sentinel, `count_level_2d`, `generate_slices`, `get_reverse_indexer`, etc. The "everything else" Cython module. |
| `missing.pyx` | 549 | 17 | 20 | Missing value handling: `checknull`, `isnaobj`, `is_matching_na`. Fast NA detection for all supported types. |
| `ops.pyx` | 310 | 53 | -- | Comparison and logical operations on object arrays: `scalar_compare`, `vec_compare`, `scalar_binop`, `vec_binop`. Handles NA propagation in object-dtype operations. |
| `ops_dispatch.pyx` | 121 | 5 | -- | Operator dispatch: `maybe_dispatch_ufunc_to_dunder_op`. Routes numpy ufuncs to pandas dunder methods. |
| `parsers.pyx` | 2,182 | 77 | -- | C-backed CSV parser interface: `TextReader` class wrapping the C tokenizer. Column type inference, NA value handling, date parsing. The bridge between C tokenizer and Python. |
| `properties.pyx` | 69 | 27 | -- | `CachedProperty` descriptor for lazy-evaluated cached attributes |
| `reshape.pyx` | 119 | 16 | -- | `unstack` helper: fast indexer generation for reshaping operations |
| `sas.pyx` | 549 | 7 | -- | SAS7BDAT binary file parser: page/subheader parsing, RLE decompression, row data extraction |
| `sparse.pyx` | 732 | 51 | -- | Sparse array operations: `BlockIndex`, `IntIndex`, `SparseIndex`, `make_mask_object_ndarray`. COO/CSR conversion helpers. |
| `testing.pyx` | 212 | 14 | -- | Assertion helpers: `assert_almost_equal`, `assert_dict_equal` with configurable tolerance |
| `tslib.pyx` | 582 | 33 | -- | Timestamp array operations: `array_with_unit_to_datetime`, `array_to_datetime`. Bulk conversion from various inputs to datetime64. |
| `writers.pyx` | 174 | 20 | -- | CSV writer helpers: `write_csv_rows`, `convert_json_to_lines` |

#### 6.2 `_libs/tslibs/` -- Time Series Library (26,418 LOC pyx)

**Purpose:** High-performance Cython implementations for all datetime/timedelta/period/offset operations. This is the most Cython-dense subpackage.

| Module | LOC (pyx) | LOC (pyi) | LOC (pxd) | Responsibility |
|--------|-----------|-----------|-----------|---------------|
| `__init__.py` | 89 (py) | -- | -- | Re-exports core time types: `Timestamp`, `Timedelta`, `Period`, `NaT`, `Interval`, frequency constants |
| `base.pyx` | 12 | -- | 5 | `ABCTimestamp` abstract base class for Cython timestamp hierarchy |
| `ccalendar.pyx` | 310 | 12 | 20 | Calendar utilities: `get_firstbday`, `get_lastbday`, `get_day_of_year`, day-of-week constants. Pure algorithmic date math. |
| `conversion.pyx` | 831 | 14 | 56 | Datetime conversion: `localize_pydatetime`, `normalize_i8_timestamps`, `tz_convert_from_utc`. Timezone-aware int64-to-datetime conversion. |
| `dtypes.pyx` | 650 | 86 | 115 | Frequency/resolution enums: `NpyDatetimeUnit`, `PeriodDtypeCode`, `attrname_to_npy_unit`, `freq_to_period_freqstr`. Maps string frequency codes to internal enums. |
| `fields.pyx` | 842 | 62 | -- | Datetime field extraction: `get_date_name_field`, `get_start_end_field`, `build_field_sarray`. Extracts year/month/day/hour/minute/second from arrays of int64 timestamps. |
| `nattype.pyx` | 1,908 | 184 | 18 | `NaT` (Not a Time) singleton: implements all datetime/timedelta methods to propagate NaT. 184 lines of stub needed because NaT must quack like both Timestamp and Timedelta. |
| `np_datetime.pyx` | 760 | 27 | 122 | NumPy datetime64 interop: `check_dts_bounds`, `py_td64_to_tdstruct`, `string_to_dts`. Low-level conversions between Python datetime objects and numpy datetime64/timedelta64. |
| `offsets.pyx` | 7,730 | 332 | 12 | **Largest Cython file.** ALL date offset classes: `BusinessDay`, `MonthEnd`, `MonthBegin`, `QuarterEnd`, `YearEnd`, `Week`, `Easter`, `CustomBusinessDay`, `CustomBusinessMonthEnd`, etc. Also the offset arithmetic engine (`apply`, `rollback`, `rollforward`). |
| `parsing.pyx` | 1,124 | 30 | 16 | Date string parsing: `parse_datetime_string_with_reso`, `dateutil_parse`, `try_parse_dates`. Central date-string-to-datetime conversion with resolution detection. |
| `period.pyx` | 3,214 | 135 | 7 | `Period` class: fiscal period arithmetic, `_Period.__new__`, `period_asfreq`, frequency conversion. Complete period object with start/end time computation. |
| `strptime.pyx` | 1,000 | 14 | 29 | `array_strptime`: vectorized strptime. Parses arrays of date strings using format codes, with NA handling and timezone support. |
| `timedeltas.pyx` | 2,762 | 168 | 28 | `Timedelta` class: nanosecond-resolution duration. Arithmetic, comparison, string representation, component extraction (days, hours, minutes, etc.). |
| `timestamps.pyx` | 3,613 | 242 | 36 | `Timestamp` class: nanosecond-resolution point in time. Subclasses `datetime.datetime`. Timezone handling, rounding, frequency snapping, component access. |
| `timezones.pyx` | 437 | 21 | 23 | Timezone utilities: `tz_compare`, `infer_tzinfo`, `maybe_get_tz`. Normalizes between pytz, dateutil, and zoneinfo timezone objects. |
| `tzconversion.pyx` | 839 | 21 | 39 | Timezone conversion: `tz_localize_to_utc`, `tz_convert_from_utc_single`. Handles ambiguous/nonexistent times during DST transitions. |
| `vectorized.pyx` | 386 | 41 | -- | Vectorized datetime operations: `ints_to_pydatetime`, `get_resolution`. Bulk conversion from int64 arrays to Python datetime objects. |

#### 6.3 `_libs/window/` -- Window Aggregation Extensions (2,269 LOC pyx)

| Module | LOC (pyx) | LOC (pyi) | Responsibility |
|--------|-----------|-----------|---------------|
| `__init__.py` | 0 (py) | -- | Empty package init |
| `aggregations.pyx` | 2,118 | 145 | Rolling/expanding window aggregation kernels: `roll_sum`, `roll_mean`, `roll_var`, `roll_skew`, `roll_kurt`, `roll_median_c`, `roll_min`/`roll_max`, `roll_quantile`, `roll_rank`, `ewm` (exponentially weighted) variants. Fixed and variable-length window support. |
| `indexers.pyx` | 151 | 12 | `calculate_variable_window_bounds`: computes start/end arrays for variable-length windows (e.g., time-based rolling windows) |

#### 6.4 `_libs/` Cython Template Files (`.pxi.in`)

| Template | LOC | Responsibility |
|----------|-----|---------------|
| `algos_common_helper.pxi.in` | 73 | Generates typed `take_1d` / `take_2d` implementations |
| `algos_take_helper.pxi.in` | 247 | Generates typed take-with-fill implementations for all numeric types |
| `hashtable_class_helper.pxi.in` | 1,572 | Generates `Int64HashTable`, `UInt64HashTable`, `Float64HashTable`, `StringHashTable`, `PyObjectHashTable` classes |
| `hashtable_func_helper.pxi.in` | 482 | Generates `unique`, `ismember`, `duplicated` for each hash table type |
| `index_class_helper.pxi.in` | 78 | Generates typed index engine specializations |
| `intervaltree.pxi.in` | 439 | Generates `IntervalTree` for int64/float64/uint64 key types |
| `khash_for_primitive_helper.pxi.in` | 44 | klib hash function specializations for primitive types |
| `sparse_op_helper.pxi.in` | 313 | Generates typed sparse array arithmetic operations |
| `free_threading_config.pxi.in` | 3 | CPython free-threading build detection |

#### 6.5 `_libs/` C Source (9,840 LOC) and Headers (2,559 LOC)

| Directory | LOC (C) | LOC (H) | Responsibility |
|-----------|---------|---------|---------------|
| `src/datetime/` | 402 | 138 | Date conversion C functions called by Cython |
| `src/parser/` | 2,255 | 364 | CSV tokenizer in C: the fastest CSV parsing path |
| `src/vendored/numpy/datetime/` | 1,995 | 205 | Vendored NumPy datetime C code for nanosecond resolution |
| `src/vendored/ujson/` | 4,988 | -- | Vendored UltraJSON encoder/decoder (C) |
| `src/vendored/klib/` | -- | 1,145 | klib hash table C header (khash) |
| `include/pandas/` | -- | 707 | Public C headers for external extension access |

---

### 7. `core/` -- The Engine (179,016 LOC)

**Layer:** 2 (core engine)
**Depends on:** `_libs`, `_config`, `errors`, `compat`, `util`, `_typing`; also `io.formats` (VIOLATION)
**Depended on by:** `io`, `plotting`, `_testing`, `api`, `tseries`

#### 7.1 Top-Level `core/` Modules

| Module | LOC | Responsibility | Key Dependencies |
|--------|-----|---------------|-----------------|
| `frame.py` | 18,679 | **DataFrame class.** The central 2D labeled data structure. Contains ~200 methods: constructors, indexing, merging, groupby access, I/O dispatch (`.to_csv()`, `.to_parquet()`, etc.), arithmetic, aggregation, iteration, reshaping. | `generic`, `series`, `internals`, `indexes`, `dtypes`, `arrays`, `io.formats`, `io.stata`, `io.parquet`, `io.feather_format`, `io.orc` |
| `generic.py` | 12,788 | **NDFrame base class.** Shared implementation for DataFrame and Series: `.loc`/`.iloc`, `.reindex()`, `.drop()`, `.rename()`, `.astype()`, `.copy()`, `.pipe()`, `.apply()`, metadata propagation, alignment logic. | `internals.managers`, `indexes`, `dtypes`, `algorithms`, `_config`, `flags` |
| `series.py` | 9,860 | **Series class.** 1D labeled array. Inherits from NDFrame. String/datetime/categorical accessor dispatch, arithmetic, comparison, aggregation. | `generic`, `indexes`, `arrays`, `dtypes`, `algorithms`, `base` |
| `algorithms.py` | 1,712 | Generic data algorithms: `unique`, `factorize`, `duplicated`, `isin`, `mode`, `rank`, `safe_sort`, `diff`. Entry points that dispatch to _libs Cython implementations. | `_libs.lib`, `_libs.hashtable`, `_libs.algos`, `dtypes` |
| `apply.py` | 2,147 | `.apply()` and `.agg()` implementation: `FrameApply`, `SeriesApply`, `GroupByApply`, `ResamplerApply`. Dispatch logic for user-defined functions. | `algorithms`, `common`, `construction` |
| `arraylike.py` | 534 | Mixin class providing array-like dunder methods (`__add__`, `__eq__`, `__neg__`, etc.) shared by Series, Index, and ExtensionArray. | `ops`, `roperator` |
| `base.py` | 1,670 | `IndexOpsMixin`: shared base for Index and Series. Provides `unique`, `nunique`, `value_counts`, `argmin`, `argmax`, `factorize`, `map`, `tolist`. | `algorithms`, `dtypes`, `arrays` |
| `accessor.py` | 511 | `register_*_accessor` machinery and `Accessor` descriptor for `.str`, `.dt`, `.cat`, `.sparse` namespace delegation. | minimal |
| `col.py` | 374 | `col()` function for deferred column references in DataFrame operations (experimental API). | minimal |
| `common.py` | 685 | Internal utilities: `maybe_box_native`, `pipe`, `random_state`, `count_not_none`, `cast_scalar_indexer`, `is_bool_indexer`. Not part of public API. | `_libs.lib`, `dtypes` |
| `config_init.py` | 923 | Registers ALL `pd.options.*` configuration options: display, mode, compute, plotting settings. Executed at import time. | `_config.config`, `io.formats.printing` |
| `construction.py` | 852 | Array construction dispatch: `array()`, `extract_array()`, `sanitize_array()`. Routes inputs to appropriate ExtensionArray or numpy array constructors. Should not depend on internals. | `arrays`, `dtypes.cast`, `dtypes.common` |
| `flags.py` | 129 | `Flags` class for per-object settings (e.g., `allows_duplicate_labels`). Attached to DataFrame/Series via weak reference. | minimal |
| `indexing.py` | 3,384 | `.loc`, `.iloc`, `.at`, `.iat` accessor implementations: `_LocIndexer`, `_iLocIndexer`, `_AtIndexer`, `_iAtIndexer`. Complex label-based and position-based indexing with slice, boolean, and fancy indexing support. | `indexes`, `dtypes`, `algorithms`, `_libs.indexing` |
| `missing.py` | 1,103 | Missing value handling: `interpolate_2d`, `clean_fill_method`, `clean_interp_method`, `interpolate_array_2d`. Fill/interpolation logic for `fillna`/`interpolate`. | `_libs.algos`, `dtypes.missing`, `algorithms` |
| `nanops.py` | 1,777 | NA-aware aggregation functions: `nansum`, `nanmean`, `nanstd`, `nanvar`, `nanmin`, `nanmax`, `nanskew`, `nankurt`, `nanprod`, `nansem`, `nanmedian`, `nanpercentile`. Skipna-aware wrappers around numpy reductions. | `_libs.missing`, `dtypes`, `compat` |
| `resample.py` | 3,188 | Resampling: `Resampler`, `DatetimeIndexResampler`, `PeriodIndexResampler`, `TimedeltaIndexResampler`. Time-based groupby-and-aggregate operations. | `groupby`, `indexes.datetimes`, `tseries.frequencies`, `tseries.offsets` |
| `roperator.py` | 63 | Reversed arithmetic operators: `radd`, `rsub`, `rmul`, `rdiv`, `rmod`, `rpow`. Used by `arraylike.py`. | none |
| `sample.py` | 163 | Sampling logic for `DataFrame.sample()` and `GroupBy.sample()`. Random row/group selection with weights. | `algorithms`, `common` |
| `sorting.py` | 736 | Sorting utilities: `nargsort`, `get_group_index`, `compress_group_index`, `get_flattened_list`, `lexsort_indexer`. Used by groupby, merge, and MultiIndex operations. | `_libs.algos`, `_libs.hashtable`, `dtypes` |

#### 7.2 `core/arrays/` -- Extension Array Types (30,774 LOC)

**Purpose:** Implements all array-level storage types. This is the "column engine" of pandas.

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 43 | Re-exports all array classes |  |
| `base.py` | 3,042 | `ExtensionArray` abstract base class. Defines the 40+ method interface that all array types must implement: `_from_sequence`, `dtype`, `__getitem__`, `__setitem__`, `take`, `copy`, `isna`, `fillna`, `unique`, `_concat_same_type`, `_reduce`, arithmetic operator hooks, etc. | `dtypes.base`, `dtypes.common`, `io.formats.printing` |
| `_mixins.py` | 631 | `NDArrayBackedExtensionArray`: shared base for arrays backed by a single ndarray (Datetime, Timedelta, Period, Categorical). Provides `_ndarray` storage and delegated operations. | `base`, `_libs.arrays` |
| `_arrow_string_mixins.py` | 421 | Mixin providing string methods (`_str_upper`, `_str_contains`, `_str_replace`, etc.) for ArrowStringArray. Delegates to PyArrow compute functions. | none (pure mixin) |
| `_ranges.py` | 209 | `generate_regular_range`: generates evenly-spaced datetime/timedelta ranges. Used by `date_range()` and `timedelta_range()`. | `_libs.tslibs` |
| `_utils.py` | 75 | Small helpers for array construction | minimal |
| `arrow/__init__.py` | 7 | Re-exports `ArrowExtensionArray` | |
| `arrow/_arrow_utils.py` | 50 | PyArrow utility functions for type mapping | `_libs.lib` |
| `arrow/accessors.py` | 501 | `.list`, `.struct`, `.dict` sub-accessors for ArrowExtensionArray columns containing nested Arrow types | `accessor` |
| `arrow/array.py` | 3,417 | `ArrowExtensionArray`: pandas ExtensionArray backed by a PyArrow ChunkedArray. Supports all Arrow-native types. The primary bridge between pandas and Arrow memory. | `base`, `masked`, `_libs`, `dtypes`, `io._util` |
| `arrow/extension_types.py` | 174 | Custom PyArrow extension types for pandas-specific semantics (e.g., `ArrowPeriodType`, `ArrowIntervalType`) | `_libs.tslibs` |
| `boolean.py` | 438 | `BooleanArray`: nullable boolean array using a mask. Inherits from `BaseMaskedArray`. | `masked`, `dtypes` |
| `categorical.py` | 3,194 | `Categorical`: ordered/unordered categorical data. Stores integer codes + Index of categories. Supports reordering, adding/removing categories, groupby optimizations. | `base`, `dtypes.dtypes`, `dtypes.cast`, `algorithms`, `io.formats` |
| `datetimelike.py` | 2,757 | `DatetimeLikeArrayMixin`: shared base for DatetimeArray, TimedeltaArray, PeriodArray. Implements frequency validation, arithmetic, comparison, shift, to_period/to_timestamp conversions. | `_mixins`, `_libs.tslibs`, `dtypes` |
| `datetimes.py` | 3,123 | `DatetimeArray`: nanosecond datetime64 storage. Timezone-aware. Implements `.tz_localize()`, `.tz_convert()`, `.normalize()`, component properties (`.year`, `.month`, `.day`, etc.). | `datetimelike`, `_libs.tslibs.timestamps`, `_libs.tslibs.conversion` |
| `floating.py` | 192 | `Float32Dtype`, `Float64Dtype` and `FloatingArray`: nullable floating-point. Thin wrapper inheriting from `numeric.py` + `masked.py`. | `numeric`, `masked` |
| `integer.py` | 296 | `Int8Dtype`..`UInt64Dtype` and `IntegerArray`: nullable integer types. Thin wrapper inheriting from `numeric.py` + `masked.py`. | `numeric`, `masked` |
| `interval.py` | 1,889 | `IntervalArray`: array of `Interval` objects. Stores left/right endpoints as separate arrays. Supports overlaps, contains, set operations. | `base`, `_libs.interval`, `dtypes.dtypes` |
| `masked.py` | 2,011 | `BaseMaskedArray`: base class for nullable integer/float/boolean. Two-array design: `_data` (values) + `_mask` (boolean NA indicator). Implements arithmetic with NA propagation. | `base`, `_libs.missing`, `dtypes`, `nanops` |
| `numeric.py` | 305 | `NumericDtype`: base for nullable numeric dtypes. Dtype registration and construction shared by integer.py and floating.py. | `masked`, `dtypes.base` |
| `numpy_.py` | 652 | `NumpyExtensionArray`: wraps a plain numpy ndarray as an ExtensionArray for the `object` dtype case. Provides EA interface over arbitrary numpy arrays. | `base`, `dtypes` |
| `period.py` | 1,493 | `PeriodArray`: array of `Period` values. Fixed-frequency fiscal period storage. `.asfreq()`, `.to_timestamp()`, `.start_time`/`.end_time`. | `datetimelike`, `_libs.tslibs.period`, `_libs.tslibs.dtypes` |
| `sparse/__init__.py` | 19 | Re-exports `SparseArray`, `SparseDtype` | |
| `sparse/accessor.py` | 500 | `.sparse` accessor: `SparseFrameAccessor`, `SparseAccessor`. Provides `.sparse.density`, `.sparse.fill_value`, `.sparse.to_coo()`, `.sparse.from_coo()`. | `accessor`, `sparse/array` |
| `sparse/array.py` | 1,993 | `SparseArray`: compressed sparse storage. Stores only non-fill values plus a `SparseIndex`. Supports arithmetic, reductions, conversion to/from scipy.sparse. | `base`, `_libs.sparse`, `dtypes` |
| `sparse/scipy_sparse.py` | 208 | Conversion between `SparseArray`/`DataFrame` and scipy.sparse COO/CSC/CSR matrices. | `scipy.sparse` |
| `string_.py` | 1,232 | `StringDtype` and `StringArray`: dedicated string type using Python object storage or (optionally) PyArrow backing. Implements NA-aware string operations. | `base`, `masked`, `_libs.lib`, `io.formats.printing` |
| `string_arrow.py` | 592 | `ArrowStringArray`: string array backed by PyArrow `large_string` type. Higher performance for string operations via Arrow compute. | `arrow/array`, `_arrow_string_mixins` |
| `timedeltas.py` | 1,310 | `TimedeltaArray`: nanosecond timedelta64 storage. Arithmetic, total_seconds, component properties. | `datetimelike`, `_libs.tslibs.timedeltas` |

#### 7.3 `core/dtypes/` -- Type System (9,102 LOC)

**Purpose:** Type detection, casting, missing value logic, and ExtensionDtype definitions. The "type algebra" of pandas.

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 0 | Empty | |
| `api.py` | 83 | Public API re-exports: `is_bool_dtype`, `is_integer_dtype`, etc. | `common` |
| `astype.py` | 328 | Type casting: `astype_array`, `astype_is_view`. Implements the conversion logic for `.astype()`. | `common`, `_libs.lib` |
| `base.py` | 597 | `ExtensionDtype` abstract base class. Defines the interface for custom dtypes: `name`, `type`, `na_value`, `construct_array_type()`, registry. | minimal |
| `cast.py` | 1,879 | Casting utilities: `maybe_downcast_to_dtype`, `maybe_upcast`, `infer_dtype_from_scalar`, `maybe_convert_objects`, `convert_dtypes`, `construct_1d_object_array_from_listlike`, `find_common_type`. The workhorse of dtype coercion. | `_libs.lib`, `_libs.tslibs`, `common`, `io._util` (VIOLATION) |
| `common.py` | 1,988 | Type checking predicates: `is_integer_dtype`, `is_float_dtype`, `is_bool_dtype`, `is_datetime64_dtype`, `is_categorical_dtype`, `is_extension_array_dtype`, `is_string_dtype`, `pandas_dtype()`, etc. ~80 public type-checking functions. | `_libs.lib`, `dtypes`, `generic` |
| `concat.py` | 342 | `find_common_type`, `concat_compat`: dtype negotiation for `pd.concat()`. Determines the result dtype when concatenating arrays of different types. | `common`, `cast`, `_libs` |
| `dtypes.py` | 2,480 | Concrete ExtensionDtype implementations: `CategoricalDtype`, `DatetimeTZDtype`, `PeriodDtype`, `IntervalDtype`, `SparseDtype`, `ArrowDtype`, `NumpyEADtype`. Dtype metadata + construction + hash + equality. | `base`, `_libs.tslibs`, `_libs.lib` |
| `generic.py` | 148 | ABCMeta-based generic type definitions: `ABCDataFrame`, `ABCSeries`, `ABCIndex`, `ABCMultiIndex`, `ABCCategorical`, etc. Used for isinstance checks without circular imports. | minimal |
| `inference.py` | 518 | Type inference: `is_list_like`, `is_dict_like`, `is_array_like`, `is_sequence`, `is_iterator`, `is_re`, `is_hashable`, `is_number`, `is_float`, `is_integer`, `is_bool`, `is_scalar`. | `_libs.lib` |
| `missing.py` | 739 | Missing value utilities: `isna`, `notna`, `array_equivalent`, `is_valid_na_for_dtype`, `na_value_for_dtype`. Central NA detection for all dtypes. | `_libs.missing`, `_libs.lib`, `common` |

#### 7.4 `core/indexes/` -- Index Types (21,816 LOC)

**Purpose:** All index (axis label) implementations. Indexes provide O(1) label-based lookup and alignment.

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 0 | Empty | |
| `accessors.py` | 711 | `.dt` and `.str` sub-accessors for Index objects. Delegates to underlying array's accessor. | `arrays.datetimes`, `arrays.timedeltas`, `arrays.period` |
| `api.py` | 342 | Re-exports all Index classes + `get_objs_combined_axis`, `all_indexes_same` | |
| `base.py` | 8,082 | **Index base class.** O(1) label lookup via `_engine` (Cython hash table). Implements `get_loc`, `get_indexer`, `reindex`, `union`, `intersection`, `difference`, `join`, `append`, `sort_values`, `unique`, `duplicated`, `isin`, set operations. The foundation of alignment. | `_libs.index`, `_libs.lib`, `dtypes`, `arrays`, `io.formats.printing` |
| `category.py` | 552 | `CategoricalIndex`: index backed by `Categorical` array. Optimized `get_indexer` for categorical codes. | `base`, `arrays.categorical` |
| `datetimelike.py` | 1,143 | `DatetimeTimedeltaMixin`: shared base for `DatetimeIndex` and `TimedeltaIndex`. Frequency validation, slicing, shift. | `base`, `_libs.tslibs` |
| `datetimes.py` | 1,618 | `DatetimeIndex`: datetime64 index with timezone support. `date_range()` constructor. Optimized slicing, frequency inference, partial string indexing (e.g., `df['2020']`). | `datetimelike`, `arrays.datetimes`, `_libs.tslibs`, `io.formats.format` |
| `extension.py` | 176 | `ExtensionIndex`: base class for indexes backed by any ExtensionArray. Thin delegation. | `base` |
| `frozen.py` | 121 | `FrozenList`: immutable list used for MultiIndex `.names` and `.levels`. Prevents accidental mutation. | `io.formats.printing` |
| `interval.py` | 1,470 | `IntervalIndex`: index of `Interval` objects. Supports `get_loc` with overlap semantics, `overlaps`, `contains`. Backed by `IntervalTree`. | `base`, `arrays.interval`, `_libs.interval` |
| `multi.py` | 4,807 | **MultiIndex**: hierarchical (multi-level) index. Stores `levels` + `codes`. Implements `get_loc` with partial key matching, `get_locs`, cross-level operations, `droplevel`, `swaplevel`. The most complex Index subclass. | `base`, `frozen`, `sorting`, `_libs.algos`, `_libs.index`, `_libs.lib` |
| `period.py` | 793 | `PeriodIndex`: index of `Period` values. `period_range()` constructor. Frequency-based slicing and alignment. | `datetimelike`, `arrays.period`, `_libs.tslibs.period` |
| `range.py` | 1,584 | `RangeIndex`: memory-efficient integer index stored as start/stop/step (no array allocation). Optimized arithmetic and set operations. Default index type for DataFrames. | `base`, `_libs.lib` |
| `timedeltas.py` | 417 | `TimedeltaIndex`: timedelta64 index. `timedelta_range()` constructor. | `datetimelike`, `arrays.timedeltas` |

#### 7.5 `core/internals/` -- Block Manager (6,874 LOC)

**Purpose:** The storage layer that manages the internal columnar blocks of a DataFrame. This is the "guts" of DataFrame memory layout.

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 65 | Re-exports `BlockManager`, `SingleBlockManager`, `make_block` | |
| `api.py` | 177 | Pseudo-public API for downstream libraries needing internals access. `make_block`, `_maybe_infer_ndim`. Stability promise for external consumption. | `blocks` |
| `blocks.py` | 2,395 | **Block classes.** `Block`, `ExtensionBlock`, `DatetimeLikeBlock`, `ObjectBlock`. Each block stores a 2D numpy array (or ExtensionArray) representing one or more same-dtype columns. Implements `setitem`, `putmask`, `where`, `fillna`, `interpolate`, `astype`, `shift` at the block level. | `dtypes`, `arrays`, `_libs.internals`, `_libs.missing`, `array_algos` |
| `concat.py` | 479 | `concatenate_managers`: concatenates BlockManagers for `pd.concat()`. Determines block structure of the result. | `blocks`, `dtypes.concat` |
| `construction.py` | 1,055 | `arrays_to_mgr`, `dict_to_mgr`, `rec_array_to_mgr`, `ndarray_to_mgr`: converts various inputs into a BlockManager during DataFrame/Series construction. | `blocks`, `managers`, `indexes.api`, `dtypes`, `construction` (core) |
| `managers.py` | 2,548 | **BlockManager** and **SingleBlockManager**: manages the collection of blocks. `apply()` (block-wise operation dispatch), `get_dtypes`, `as_array`, `consolidate`, `reindex_axis`, `take`. This is the core data container that DataFrame wraps. | `blocks`, `_libs.internals`, `_libs.lib`, `dtypes`, `indexes` |
| `ops.py` | 155 | `operate_blockwise`: applies a binary operation across two BlockManagers column-by-column. Used for DataFrame arithmetic. | `blocks` |

#### 7.6 `core/groupby/` -- GroupBy (13,045 LOC)

**Purpose:** Split-apply-combine framework. One of the most used pandas features.

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 15 | Re-exports `GroupBy`, `DataFrameGroupBy`, `SeriesGroupBy`, `NamedAgg` | |
| `base.py` | 121 | GroupBy base constants: `OutputKey`, common strings, `no_arg` sentinel | minimal |
| `categorical.py` | 83 | `recode_for_groupby`, `recode_from_groupby`: remaps categorical codes for groupby operations, handling observed/unobserved categories. | `algorithms`, `arrays.categorical` |
| `generic.py` | 3,987 | `DataFrameGroupBy` and `SeriesGroupBy`: concrete GroupBy classes with column-specific methods. `.agg()`, `.transform()`, `.filter()`, `.value_counts()`, `.describe()`, `.fillna()`, `.resample()`. | `groupby`, `frame`, `series`, `algorithms`, `dtypes` |
| `groupby.py` | 6,036 | **GroupBy base class.** Core split-apply-combine engine. `_iterate_slices`, `_cython_agg_general`, `_agg_py_fallback`, `_python_agg_general`, `nth`, `cumcount`, `ngroup`, `cummax`, `cummin`, `cumprod`, `cumsum`, `shift`, `diff`, `pct_change`, `head`, `tail`, `sample`, `pipe`, `plot`. | `ops`, `_libs.groupby`, `_libs.lib`, `algorithms`, `sorting`, `indexes`, `dtypes` |
| `grouper.py` | 961 | `Grouper`, `Grouping`: constructs grouping metadata from user input (column names, functions, Grouper objects, TimeGrouper). Resolves what the "groups" are. | `algorithms`, `indexes`, `arrays.categorical` |
| `indexing.py` | 378 | `GroupByIndexingMixin`: provides `.iloc` positional indexing within groups. | minimal |
| `numba_.py` | 183 | Numba JIT compilation for custom groupby aggregation functions. Generates numba-compatible wrappers. | `_numba`, `compat._optional` |
| `ops.py` | 1,281 | `WrappedCythonOp`, `BaseGrouper`, `BinGrouper`: low-level groupby execution engine. Maps operation names to Cython kernels. Manages splitter/agg dispatch. | `_libs.groupby`, `_libs.lib`, `algorithms`, `sorting` |

#### 7.7 `core/reshape/` -- Reshaping (8,555 LOC)

**Purpose:** Data restructuring operations: merge, concat, pivot, melt, stack/unstack, get_dummies.

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 0 | Empty | |
| `api.py` | 41 | Re-exports: `concat`, `merge`, `merge_ordered`, `merge_asof`, `pivot`, `pivot_table`, `get_dummies`, `from_dummies`, `lreshape`, `melt`, `wide_to_long`, `cut`, `qcut`, `crosstab` | |
| `concat.py` | 998 | `pd.concat()`: concatenation of DataFrame/Series along an axis. `Concatenator` class handles index alignment, block concatenation, and dtype unification. | `internals.concat`, `indexes.api`, `dtypes`, `algorithms` |
| `encoding.py` | 589 | `get_dummies()` and `from_dummies()`: one-hot encoding. Converts categorical columns to binary indicator columns and back. | `arrays.categorical`, `dtypes`, `algorithms` |
| `melt.py` | 676 | `melt()` / `wide_to_long()` / `lreshape()`: unpivoting operations. Converts wide-format to long-format. | `indexes`, `dtypes` |
| `merge.py` | 3,135 | **`pd.merge()`**: the join engine. `_MergeOperation`, `_AsofMerge`, `_OrderedMerge`. Left/right/inner/outer/cross joins, key validation, join-key coercion, sort, indicator columns. | `_libs.join`, `_libs.hashtable`, `sorting`, `indexes`, `dtypes`, `algorithms` |
| `pivot.py` | 1,310 | `pivot_table()` and `pivot()` / `crosstab()`: aggregation-based reshaping. Creates multi-level column structures from grouped aggregations. | `groupby`, `concat`, `frame`, `dtypes`, `algorithms` |
| `reshape.py` | 1,124 | `stack()` / `unstack()` / `get_dummies()` core logic: converts between stacked (long) and unstacked (wide) MultiIndex layouts. | `_libs.reshape`, `indexes.multi`, `internals`, `dtypes` |
| `tile.py` | 682 | `pd.cut()` / `pd.qcut()`: discretization/binning. Converts continuous values to categorical intervals. | `arrays.interval`, `algorithms`, `dtypes` |

#### 7.8 `core/window/` -- Window Operations (6,771 LOC)

**Purpose:** Rolling, expanding, and exponentially weighted (EWM) computations.

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 23 | Re-exports `Expanding`, `ExponentialMovingWindow`, `Rolling`, `Window` | |
| `common.py` | 171 | Shared window utilities: `flex_binary_moment`, `zsqrt`, `prep_binary` | minimal |
| `ewm.py` | 1,175 | `ExponentialMovingWindow`: `.ewm()` accessor. Computes exponentially weighted mean, std, var, cov, corr with configurable span/halflife/alpha. | `_libs.window.aggregations`, `common`, `rolling` |
| `expanding.py` | 1,412 | `Expanding`: `.expanding()` accessor. Cumulative window that grows from start. Inherits most logic from `Rolling` but with `min_periods=1` semantics. | `rolling`, `_libs.window.aggregations` |
| `numba_.py` | 357 | Numba JIT compilation for custom window aggregation functions. Generates sliding-window numba wrappers. | `_numba`, `compat._optional` |
| `online.py` | 117 | `OnlineExponentialMovingWindow`: streaming EWM computation that updates incrementally with new data. | `ewm` |
| `rolling.py` | 3,516 | **`Rolling`** and **`Window`**: `.rolling()` accessor. Fixed and variable-length window operations. `mean`, `sum`, `std`, `var`, `min`, `max`, `median`, `quantile`, `skew`, `kurt`, `corr`, `cov`, `apply`, `rank`. Also `Window` for weighted windows (Gaussian, triangular, etc.). | `_libs.window.aggregations`, `_libs.window.indexers`, `indexers.objects`, `common`, `dtypes` |

#### 7.9 `core/ops/` -- Operator Dispatch (1,347 LOC)

**Purpose:** Arithmetic and comparison operator implementation and dispatch.

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 92 | Re-exports: `comp_method_OBJECT_ARRAY`, common operators | |
| `array_ops.py` | 620 | `arithmetic_op`, `comparison_op`, `logical_op`: core binary operation dispatch. Routes operations based on array types, handles alignment, NA propagation, and type promotion. | `_libs.ops`, `_libs.ops_dispatch`, `dtypes`, `construction`, `roperator` |
| `common.py` | 157 | `unpack_zerodim_and_defer`: dunder method wrapper that handles 0-d arrays and defers to `__rop__` methods. | `_libs.lib` |
| `dispatch.py` | 31 | `should_extension_dispatch`: decides whether to dispatch an operation to ExtensionArray or fall back to numpy. | `dtypes.generic` |
| `invalid.py` | 77 | `invalid_comparison`: generates appropriate error/result for comparisons that cannot be performed (e.g., comparing datetime with int). | `dtypes` |
| `mask_ops.py` | 194 | `kleene_or`, `kleene_and`, `kleene_xor`: three-valued logic operations for nullable boolean arrays (True/False/NA). | `_libs.missing`, `_libs.lib` |
| `missing.py` | 176 | `dispatch_fill_zeros`: handles division-by-zero semantics (fill with inf/nan/0 depending on operation). | `_libs.ops` |

#### 7.10 `core/strings/` -- String Methods (5,433 LOC)

**Purpose:** `.str` accessor and vectorized string operations.

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 24 | Re-exports `StringMethods` | |
| `accessor.py` | 4,877 | **`StringMethods`**: the `.str` accessor class. 50+ vectorized string operations: `contains`, `match`, `extract`, `extractall`, `replace`, `split`, `join`, `strip`, `pad`, `center`, `lower`, `upper`, `title`, `find`, `count`, `startswith`, `endswith`, `slice`, `get`, `get_dummies`, `encode`, `decode`, etc. | `base`, `arrays`, `dtypes`, `construction` |
| `object_array.py` | 532 | `ObjectStringArrayMixin`: default (Python object-backed) implementations of string operations. Fallback when not using Arrow-backed strings. | `_libs.lib`, `_libs.missing` |

#### 7.11 `core/computation/` -- Expression Evaluation (3,877 LOC)

**Purpose:** `pd.eval()` and `DataFrame.query()` / `DataFrame.eval()` expression engine.

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 0 | Empty | |
| `align.py` | 227 | Aligns operands in expressions to common shape/index before evaluation. | `indexes`, `generic` |
| `api.py` | 2 | Re-exports `eval` | |
| `check.py` | 8 | Numexpr availability check | `compat._optional` |
| `common.py` | 48 | Shared helpers: `result_type_many` for result dtype negotiation | minimal |
| `engines.py` | 151 | `NumExprEngine`, `PythonEngine`: evaluation backends. NumExpr for fast vectorized expression evaluation, Python as fallback. | `io.formats.printing` (VIOLATION) |
| `eval.py` | 454 | `pd.eval()`: top-level expression evaluator. Parses expression string, constructs AST, aligns operands, evaluates via engine. | `expr`, `engines`, `parsing`, `scope` |
| `expr.py` | 850 | Expression AST construction: `Expr`, `BinOp`, `UnaryOp`, `Cmp`, `Term`, `Constant`. Parses pandas expression syntax into an evaluable tree. | `ops`, `scope`, `io.formats.printing` (VIOLATION) |
| `expressions.py` | 290 | `evaluate()`: dispatches element-wise operations to numexpr if available, numpy otherwise. Used throughout pandas for fast binary operations. | `compat._optional` |
| `ops.py` | 561 | Expression operation types: `BinOp`, `UnaryOp`, `Cmp`, `MathCall`. Defines valid operations and their behavior in the expression evaluator. | `dtypes`, `io.formats.printing` (VIOLATION) |
| `parsing.py` | 252 | Expression tokenizer: converts expression strings to token streams. Handles operator precedence and pandas-specific syntax. | minimal |
| `pytables.py` | 678 | PyTables (HDF5) query expression compiler: translates pandas query syntax into PyTables `where` conditions for server-side filtering. | `expr`, `ops`, `scope`, `io.formats.printing` (VIOLATION) |
| `scope.py` | 356 | `Scope`: manages variable resolution for expression evaluation. Looks up names in local/global scopes, resolvers, and the calling frame. | minimal |

#### 7.12 `core/tools/` -- Conversion Tools (1,939 LOC)

**Purpose:** Functions for converting inputs to pandas scalar types.

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 0 | Empty | |
| `datetimes.py` | 1,213 | `pd.to_datetime()`: converts strings, epochs, lists, Series to datetime. The primary datetime parsing entry point. | `_libs.tslibs`, `arrays.datetimes`, `indexes.datetimes`, `dtypes` |
| `numeric.py` | 327 | `pd.to_numeric()`: converts strings/mixed to numeric. Handles `errors='coerce'` for NA-on-failure. | `_libs.lib`, `arrays`, `dtypes` |
| `timedeltas.py` | 246 | `pd.to_timedelta()`: converts strings/numbers to timedelta. | `_libs.tslibs.timedeltas`, `arrays.timedeltas` |
| `times.py` | 153 | `pd.to_time()` (internal, not in public API): converts strings to `datetime.time` objects. | `_libs.lib` |

#### 7.13 `core/indexers/` -- Indexer Utilities (1,317 LOC)

**Purpose:** Window indexer objects and indexing utilities.

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 33 | Re-exports `check_array_indexer` | |
| `objects.py` | 689 | Window indexer classes: `BaseIndexer`, `FixedWindowIndexer`, `VariableWindowIndexer`, `FixedForwardWindowIndexer`, `ExpandingIndexer`, `GroupbyIndexer`. Define start/end bounds for window computations. | `_libs.window.indexers` |
| `utils.py` | 595 | Indexing validation: `check_array_indexer`, `length_of_indexer`, `validate_indices`, `maybe_convert_indices`, `disallow_ndim_indexing`. Shared indexing safety checks. | `dtypes`, `_libs.lib` |

#### 7.14 `core/interchange/` -- DataFrame Interchange Protocol (1,991 LOC)

**Purpose:** Implementation of the Python DataFrame interchange protocol (PEP-proposed cross-library DataFrame exchange).

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 0 | Empty | |
| `buffer.py` | 135 | `PandasBuffer`: wraps ndarray as interchange `Buffer` | minimal |
| `column.py` | 479 | `PandasColumn`: wraps a pandas column as an interchange `Column` with dtype, null info, chunk support | `dtypes`, `arrays` |
| `dataframe.py` | 113 | `PandasDataFrameXchg`: wraps DataFrame as interchange `DataFrame` | `column` |
| `dataframe_protocol.py` | 468 | Protocol ABC definitions: `Buffer`, `Column`, `DataFrame`, `DtypeKind`, `ColumnNullType`. The abstract interface spec. | minimal |
| `from_dataframe.py` | 613 | `from_dataframe()`: constructs a pandas DataFrame from any interchange-protocol-compliant object. Dtype mapping and buffer copying. | `dtypes`, `arrays`, `construction` |
| `utils.py` | 183 | Interchange dtype mapping utilities | `dtypes` |

#### 7.15 `core/methods/` -- Extracted Methods (964 LOC)

**Purpose:** Methods extracted from NDFrame/DataFrame/Series for maintainability.

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 0 | Empty | |
| `describe.py` | 375 | `describe_ndframe()`: implements `.describe()`. Computes summary statistics (count, mean, std, min, percentiles, max) with dtype-appropriate behavior. | `dtypes`, `algorithms`, `indexes`, `frame` |
| `selectn.py` | 300 | `SelectNFrame`, `SelectNSeries`: `.nlargest()` and `.nsmallest()` implementations using partial sort. | `algorithms`, `dtypes` |
| `to_dict.py` | 289 | `.to_dict()` implementation: converts DataFrame/Series to Python dict with various orientations (dict, list, series, split, tight, records, index). | `_libs.lib` |

#### 7.16 `core/_numba/` -- Numba Integration (1,770 LOC)

**Purpose:** JIT-compilation infrastructure for custom aggregation functions.

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 0 | Empty | |
| `executor.py` | 246 | `generate_shared_aggregator`: creates numba-jitted aggregation function wrappers. | `kernels`, `compat._optional` |
| `extensions.py` | 586 | Numba extension types for pandas: teaches numba how to handle pandas `Index`, `Series`, and `Timestamp` objects in JIT context. | `_libs.tslibs`, `compat._optional` |
| `kernels/__init__.py` | 27 | Re-exports numba kernel functions | |
| `kernels/mean_.py` | 198 | Numba-jitted sliding window mean kernel | `shared` |
| `kernels/min_max_.py` | 178 | Numba-jitted sliding window min/max kernel | `shared` |
| `kernels/shared.py` | 29 | Shared constants for numba kernels (e.g., `is_monotonic_increasing` check) | minimal |
| `kernels/sum_.py` | 255 | Numba-jitted sliding window sum kernel | `shared` |
| `kernels/var_.py` | 251 | Numba-jitted sliding window variance kernel | `shared` |

#### 7.17 `core/array_algos/` -- Array Algorithm Dispatch (1,509 LOC)

**Purpose:** Array-level algorithm implementations that can operate on both numpy and ExtensionArrays.

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 9 | Empty | |
| `datetimelike_accumulations.py` | 73 | `cumsum` for datetime-like arrays (handles NaT) | `dtypes.missing` |
| `masked_accumulations.py` | 97 | `cumsum`, `cumprod`, `cummin`, `cummax` for `BaseMaskedArray` | `dtypes.missing` |
| `masked_reductions.py` | 201 | `sum`, `prod`, `mean`, `var`, `std`, `min`, `max` for `BaseMaskedArray`. NA-aware reductions on the mask+data pair. | `_libs.missing` |
| `putmask.py` | 150 | `putmask_inplace`, `putmask_without_repeat`: array mutation with boolean masking, handling ExtensionArray and numpy paths. | `_libs.lib`, `dtypes` |
| `quantile.py` | 226 | `quantile_with_mask`: quantile computation for masked/non-masked arrays. Handles interpolation methods. | `_libs.lib`, `dtypes` |
| `replace.py` | 157 | `compare_or_regex_search`, `_check_comparison_types`: pattern matching for `Series.replace()`. Regex and exact match paths. | `_libs.lib`, `dtypes.missing` |
| `take.py` | 546 | `take`, `take_nd`: fancy indexing for arrays. Handles fill values, out-of-bounds, and ExtensionArray dispatch. The core of `__getitem__` for integer array indexers. | `_libs.algos`, `dtypes` |
| `transforms.py` | 50 | `shift`: array-level shift with fill value | minimal |

#### 7.18 `core/sparse/` -- Sparse API (5 LOC)

| Module | LOC | Responsibility |
|--------|-----|---------------|
| `__init__.py` | 0 | Empty |
| `api.py` | 5 | Re-exports from `arrays.sparse` (compatibility stub) |

#### 7.19 `core/util/` -- Core Utilities (511 LOC)

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 0 | Empty | |
| `hashing.py` | 361 | `hash_pandas_object`, `hash_array`: deterministic hashing of pandas objects for consistent `__hash__` behavior and hash-join support. | `_libs.hashing`, `_libs.lib`, `dtypes` |
| `numba_.py` | 150 | `maybe_use_numba`, `get_jit_arguments`: shared numba utility functions for groupby/window numba dispatch. | `compat._optional` |

---

### 8. `io/` -- Input/Output (47,386 LOC)

**Layer:** 3 (I/O)
**Depends on:** `core`, `_libs`, `util`, `errors`, `compat`, `_config`
**Depended on by:** `plotting` (for format utilities), `api`, `_testing`

#### 8.1 Top-Level `io/` Modules

| Module | LOC | Responsibility | Key Depends On |
|--------|-----|---------------|---------------|
| `__init__.py` | 13 | Package init | |
| `_util.py` | 191 | I/O utilities: `_arrow_dtype_mapping` (Arrow-to-pandas dtype map), `arrow_string_types_mapper` | `_libs`, `core.dtypes` |
| `api.py` | 65 | Re-exports all read/write functions: `read_csv`, `read_excel`, `read_json`, `to_pickle`, etc. | |
| `clipboards.py` | 200 | `read_clipboard()` / `to_clipboard()`: clipboard I/O via xclip/xsel/pbcopy | `parsers.readers`, `errors` |
| `common.py` | 1,327 | Shared I/O infrastructure: `get_handle` (file/URL/buffer opening), `_compression_to_extension`, path handling, URL detection, compression auto-detection. Used by ALL I/O modules. | `compat`, `core.dtypes` |
| `feather_format.py` | 181 | `read_feather()` / `to_feather()`: Apache Feather (Arrow IPC) format | `common`, `compat`, `core` |
| `html.py` | 1,245 | `read_html()`: HTML table scraper. Parses `<table>` elements from HTML using lxml or html5lib. | `common`, `parsers`, `compat` |
| `iceberg.py` | 155 | `read_iceberg()`: Apache Iceberg table reader (experimental) | `compat` |
| `orc.py` | 243 | `read_orc()` / `to_orc()`: Apache ORC columnar format | `common`, `compat`, `_libs` |
| `parquet.py` | 680 | `read_parquet()` / `to_parquet()`: Apache Parquet columnar format. Supports PyArrow and fastparquet engines. | `common`, `compat`, `_libs` |
| `pickle.py` | 239 | `read_pickle()` / `to_pickle()`: Python pickle serialization with compression | `common`, `compat` |
| `pytables.py` | 5,595 | **`read_hdf()` / `HDFStore`**: HDF5 file reader/writer via PyTables. Supports fixed and table formats, querying, append mode. Most complex single I/O module. | `common`, `compat`, `core` (heavy), `_libs`, `computation.pytables` |
| `spss.py` | 95 | `read_spss()`: SPSS `.sav` file reader via pyreadstat | `common`, `compat`, `_libs` |
| `sql.py` | 2,960 | `read_sql()` / `read_sql_table()` / `read_sql_query()` / `to_sql()`: SQL database I/O via SQLAlchemy or ADBC. Handles schema inference, chunked reading, dtype mapping. | `common`, `core`, `_libs`, `compat` |
| `stata.py` | 3,934 | `read_stata()` / `to_stata()`: Stata `.dta` binary format. Complete Stata 104-119 format parser/writer. Category label handling, value label maps, timestamp conversion. | `common`, `core`, `_libs` |
| `xml.py` | 1,155 | `read_xml()`: XML parser (lxml/etree). XPath-based element extraction to DataFrame. | `common`, `compat`, `_libs` |

#### 8.2 `io/clipboard/` (747 LOC)

| Module | LOC | Responsibility |
|--------|-----|---------------|
| `__init__.py` | 747 | Clipboard read/write: detects platform clipboard tool, handles TSV/CSV interchange with clipboard |

#### 8.3 `io/excel/` -- Excel I/O (4,183 LOC)

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 19 | Re-exports readers/writers | |
| `_base.py` | 1,875 | `BaseExcelReader` (ABC for all readers), `ExcelWriter` (base writer), `ExcelFile`. Shared sheet selection, header parsing, dtype inference. | `common`, `parsers`, `compat` |
| `_calamine.py` | 129 | `CalamineReader`: Excel reader using python-calamine (Rust-based, fast) | `_base` |
| `_odfreader.py` | 254 | `ODFReader`: OpenDocument Format (.ods) reader via odfpy | `_base` |
| `_odswriter.py` | 362 | `ODSWriter`: OpenDocument Format writer via odfpy | `_base` |
| `_openpyxl.py` | 650 | `OpenpyxlReader` / `OpenpyxlWriter`: .xlsx reader/writer via openpyxl | `_base`, `compat` |
| `_pyxlsb.py` | 131 | `PyxlsbReader`: .xlsb (binary Excel) reader via pyxlsb | `_base` |
| `_util.py` | 328 | Excel utility functions: `get_default_engine`, column-letter-to-index conversion, cell range parsing | minimal |
| `_xlrd.py` | 147 | `XlrdReader`: .xls (legacy Excel) reader via xlrd | `_base` |
| `_xlsxwriter.py` | 288 | `XlsxWriter`: .xlsx writer via XlsxWriter library | `_base` |

#### 8.4 `io/formats/` -- Output Formatting (14,213 LOC)

**Purpose:** Controls how pandas objects are displayed and serialized to text formats.

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 9 | Empty | |
| `_color_data.py` | 157 | CSS color name to hex mapping table | none |
| `console.py` | 95 | Terminal width detection for display formatting | `_config` |
| `css.py` | 425 | CSS parser for Styler: parses CSS declarations to internal representation | minimal |
| `csvs.py` | 336 | `CSVFormatter`: formats DataFrame to CSV string/file. Handles quoting, escaping, NA representation. | `_libs.writers`, `common` (io) |
| `excel.py` | 1,023 | `ExcelFormatter`: converts DataFrame to cell-by-cell representation for Excel writers. Style/formatting handling. | `css`, `printing` |
| `format.py` | 2,076 | **Core formatting engine.** `DataFrameFormatter`, `SeriesFormatter`, `GenericArrayFormatter`, `FloatArrayFormatter`, `DatetimeArrayFormatter`, `TimedeltaArrayFormatter`. Controls repr/str output for all pandas objects. | `_config`, `_libs`, `core.dtypes`, `core.arrays` |
| `html.py` | 657 | `HTMLFormatter`, `NotebookHTMLFormatter`: renders DataFrame as HTML table. Used in Jupyter notebook display. | `format`, `printing` |
| `info.py` | 848 | `DataFrameInfo`, `SeriesInfo`: `.info()` method implementation. Memory usage calculation and display. | `core.dtypes` |
| `printing.py` | 597 | String formatting utilities: `pprint_thing`, `format_object_summary`, `adjoin`. Used throughout pandas for repr. Note: this module is arguably misplaced -- it is used by core but lives in io. | minimal |
| `string.py` | 207 | `StringFormatter`: renders DataFrame as text table for terminal display. | `format` |
| `style.py` | 4,536 | **`Styler`**: CSS-based DataFrame styling. Conditional formatting, color gradients, bar charts in cells, export to HTML/LaTeX/Excel. | `style_render`, `css`, `_config` |
| `style_render.py` | 2,681 | `StylerRenderer`: rendering backend for Styler. Generates HTML/LaTeX with applied styles. Template-based rendering. | `_config`, `_libs` |
| `xml.py` | 566 | `DataFrameXMLFormatter`: renders DataFrame as XML document (lxml/etree). | `common` (io), `_libs` |

#### 8.5 `io/json/` -- JSON I/O (2,541 LOC)

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 15 | Re-exports `read_json`, `to_json` | |
| `_json.py` | 1,476 | `read_json()` / `to_json()`: JSON reader/writer. Handles orient modes (split, records, index, columns, values, table), dtype inference, date parsing. Uses ujson C extension for speed. | `common` (io), `_libs.json`, `_table_schema`, `_libs.tslibs`, `core.dtypes` |
| `_normalize.py` | 648 | `json_normalize()`: flattens semi-structured (nested) JSON into a flat DataFrame. Record path traversal and meta field extraction. | `core`, `common` (io) |
| `_table_schema.py` | 402 | Table Schema (Frictionless Data) JSON serialization. Converts DataFrame to/from JSON Table Schema format. Used by `orient='table'`. | `core.dtypes`, `core.indexes`, `_libs.tslibs` |

#### 8.6 `io/parsers/` -- CSV/Text Parsers (5,675 LOC)

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 9 | Re-exports `TextFileReader`, `TextParser` | |
| `arrow_parser_wrapper.py` | 328 | `ArrowParserWrapper`: CSV parser using PyArrow CSV reader. Fastest path for simple CSVs. | `_libs`, `core`, `compat` |
| `base_parser.py` | 997 | `ParserBase`: shared base for C and Python parsers. Header inference, column naming, dtype application, date parsing setup, NA value handling. | `_libs`, `core.dtypes`, `core.arrays`, `algorithms` |
| `c_parser_wrapper.py` | 395 | `CParserWrapper`: wraps the C tokenizer (`_libs.parsers.TextReader`). The default fast CSV parsing path. | `base_parser`, `_libs.parsers`, `core.dtypes` |
| `python_parser.py` | 1,557 | `PythonParser`, `FixedWidthFieldParser`: pure-Python CSV/fixed-width parser. Handles edge cases the C parser cannot (e.g., Python regex separators, multi-char separators). | `base_parser`, `_libs.lib` |
| `readers.py` | 2,389 | **`read_csv()` / `read_table()`**: the main entry points. `TextFileReader` class orchestrates parser selection, chunking, and option validation. 100+ parameters. | `base_parser`, `c_parser_wrapper`, `python_parser`, `arrow_parser_wrapper`, `common` (io) |

#### 8.7 `io/sas/` -- SAS Format (1,749 LOC)

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 3 | Empty | |
| `sas7bdat.py` | 738 | `SAS7BDATReader`: reads SAS7BDAT binary files. Page-level parsing, compression handling, date format conversion. | `sas_constants`, `_libs.sas`, `_libs.byteswap` |
| `sas_constants.py` | 310 | Magic numbers, compression constants, page type constants for SAS file format | none |
| `sas_xport.py` | 501 | `XportReader`: reads SAS XPORT (transport) format files. IEEE float decoding, variable metadata. | `common` (io) |
| `sasreader.py` | 197 | `read_sas()`: entry point. Auto-detects SAS7BDAT vs XPORT format. | `sas7bdat`, `sas_xport` |

---

### 9. `plotting/` -- Visualization (9,470 LOC)

**Layer:** 4 (visualization)
**Depends on:** `core`, `io.formats`, `_libs.tslibs`, `_config`, `util`
**Depended on by:** `api` (re-exports), `core` (minor -- `core.frame` dispatches `.plot()`)

#### 9.1 Top-Level `plotting/` Modules

| Module | LOC | Responsibility |
|--------|-----|---------------|
| `__init__.py` | 99 | Re-exports: `plot_params`, `boxplot`, `hist`, `scatter_matrix`, etc. |
| `_core.py` | 2,255 | `PlotAccessor`: the `.plot` accessor class. Dispatches `.plot()`, `.plot.bar()`, `.plot.hist()`, `.plot.scatter()`, etc. to the registered plotting backend (default: matplotlib). Backend registration system. |
| `_misc.py` | 780 | Standalone plotting functions: `scatter_matrix`, `radviz`, `parallel_coordinates`, `lag_plot`, `autocorrelation_plot`, `bootstrap_plot`, `andrews_curves`, `table`. |

#### 9.2 `plotting/_matplotlib/` -- Matplotlib Backend (5,845 LOC)

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 93 | Backend registration: `plot()` dispatch function | |
| `boxplot.py` | 563 | Box plot implementation: `boxplot_frame`, `boxplot_frame_groupby` | `core`, `style` |
| `converter.py` | 1,130 | Matplotlib unit converters for pandas types: `DatetimeConverter`, `TimedeltaConverter`, `PeriodConverter`. Registers pandas types with matplotlib's unit system. | `_libs.tslibs`, `core.indexes` |
| `core.py` | 2,207 | **Core plotting classes.** `MPLPlot`, `LinePlot`, `BarPlot`, `BarhPlot`, `AreaPlot`, `PiePlot`, `ScatterPlot`, `HexBinPlot`. Base plot construction, legend handling, axis formatting. | `core`, `style`, `tools` |
| `groupby.py` | 141 | GroupBy-aware plotting: creates subplots per group | `core` (plotting) |
| `hist.py` | 574 | Histogram plotting: `hist_series`, `hist_frame` | `core` (plotting), `tools` |
| `misc.py` | 480 | Miscellaneous plot types: `scatter_matrix`, `radviz`, `parallel_coordinates`, `andrews_curves`, `lag_plot`, `autocorrelation_plot`, `bootstrap_plot` | `core` (plotting) |
| `style.py` | 293 | Color and style utilities: color cycle generation, color normalization, style keyword handling | minimal |
| `timeseries.py` | 364 | Time series plot formatting: frequency-aware x-axis ticking, period-to-datetime conversion for plotting | `_libs.tslibs`, `core.indexes`, `tseries.frequencies` |
| `tools.py` | 491 | Layout utilities: `_subplots()` (creates figure/axes grid), `create_subplots`, `flatten_axes`, `handle_shared_axes` | minimal |

---

### 10. `tseries/` -- Time Series (1,426 LOC)

**Layer:** 2 (core extension)
**Depends on:** `_libs.tslibs`, `core`, `errors`
**Depended on by:** `core.resample`, `io.json`, `plotting`

| Module | LOC | Responsibility | Depends On |
|--------|-----|---------------|-----------|
| `__init__.py` | 12 | Package init | |
| `api.py` | 10 | Re-exports `offsets` | |
| `frequencies.py` | 623 | Frequency inference: `infer_freq()`. Analyses a DatetimeIndex to determine its frequency (e.g., 'D', 'BMS', 'QS'). Complex heuristic for irregular frequencies. | `_libs.tslibs`, `core.dtypes`, `core.algorithms` |
| `holiday.py` | 682 | Holiday calendar system: `AbstractHolidayCalendar`, `Holiday`, `USFederalHolidayCalendar`, `GoodFriday`, `USMemorialDay`, etc. Computes holiday dates for business day offset calculations. | `_libs.tslibs.offsets`, `errors` |
| `offsets.py` | 99 | Re-exports ALL offset classes from `_libs.tslibs.offsets` (convenience module). DateOffset, BusinessDay, MonthEnd, etc. | `_libs.tslibs.offsets` |

---

### 11. `api/` -- Public API Surface (375 LOC)

**Layer:** 5 (public surface)
**Depends on:** `core`, `_libs`, `io`, `_typing`
**Depended on by:** external consumers

| Module | LOC | Responsibility |
|--------|-----|---------------|
| `__init__.py` | 19 | Aggregates sub-namespaces: `api.types`, `api.indexers`, `api.extensions`, `api.interchange`, `api.typing`, `api.executors` |
| `executors/__init__.py` | 7 | Exposes `ThreadPoolExecutor` for `read_csv(executor=...)` |
| `extensions/__init__.py` | 33 | Public extension API: `ExtensionDtype`, `ExtensionArray`, `register_extension_dtype`, `register_dataframe_accessor`, etc. |
| `indexers/__init__.py` | 17 | Public indexer API: `BaseIndexer`, `FixedForwardWindowIndexer`, `VariableOffsetWindowIndexer`, `check_array_indexer` |
| `interchange/__init__.py` | 8 | Exposes `from_dataframe` for DataFrame interchange protocol |
| `internals.py` | 62 | Pseudo-public internals API for downstream libraries (Dask, Modin) |
| `types/__init__.py` | 23 | Public type-checking API: wraps `core.dtypes.api` and `core.dtypes.inference` |
| `typing/__init__.py` | 61 | Public typing namespace: re-exports `DataFrame`, `Series`, `Index`, `Timestamp`, `Timedelta`, `Period`, `Interval`, etc. as typing-checkable names |
| `typing/aliases.py` | 145 | Type alias definitions for public consumption |

---

### 12. `arrays/` -- Array Re-export (37 LOC)

**Layer:** 5 (public surface)

| Module | LOC | Responsibility |
|--------|-----|---------------|
| `__init__.py` | 37 | Re-exports all public ExtensionArray types: `BooleanArray`, `IntegerArray`, `FloatingArray`, `StringArray`, `ArrowExtensionArray`, `Categorical`, `SparseArray`, `DatetimeArray`, `TimedeltaArray`, `PeriodArray`, `IntervalArray`, `NumpyExtensionArray` |

---

### 13. `_testing/` -- Test Utilities (2,813 LOC)

**Layer:** testing (cross-cutting)
**Depends on:** `core`, `_libs`, `compat`, `_config`, `errors`, `io`, `tseries`

| Module | LOC | Responsibility |
|--------|-----|---------------|
| `__init__.py` | 645 | Main entry: `makeDataFrame`, `makeDateIndex`, `makeTimeSeries`, fixture factories, index generators, data generators for tests |
| `_hypothesis.py` | 89 | Hypothesis strategies for property-based testing: `DATETIME_JAN_1_1900_PLUS_TZ`, `DATETIME_IN_PD_TIMESTAMP_RANGE_NO_TZ` |
| `_io.py` | 129 | I/O test helpers: `round_trip_pickle`, `round_trip_pathlib`, network connectivity checks |
| `_warnings.py` | 266 | Warning assertion context managers: `assert_produces_warning`, `_assert_caught_expected_warning` |
| `asserters.py` | 1,503 | **Core test assertions.** `assert_frame_equal`, `assert_series_equal`, `assert_index_equal`, `assert_extension_array_equal`, `assert_is_valid_plot_return_type`. Deep structural + value comparison with tolerance. |
| `compat.py` | 30 | Test compatibility helpers: `get_dtype` |
| `contexts.py` | 151 | Test context managers: `decompress_file`, `set_timezone`, `ensure_clean` (temp file cleanup) |

---

## Cross-Package Dependency Summary

### Dependency Direction (Allowed vs. Observed)

| Direction | Status | Count | Notes |
|-----------|--------|-------|-------|
| `core -> _libs` | ALLOWED | 192 | Core engine uses Cython extensions |
| `core -> errors` | ALLOWED | 55 | Core raises pandas exceptions |
| `core -> compat` | ALLOWED | 51 | Core uses compatibility shims |
| `core -> util` | ALLOWED | 121 | Core uses decorators/validators |
| `core -> _config` | ALLOWED | 31 | Core reads configuration |
| `io -> core` | ALLOWED | 92 | I/O constructs DataFrames |
| `io -> _libs` | ALLOWED | 41 | I/O uses Cython parsers |
| `plotting -> core` | ALLOWED | 22 | Plotting reads DataFrames |
| `tseries -> _libs` | ALLOWED | 11 | Time series uses Cython offsets |
| **core -> io** | **VIOLATION** | **22** | Formatting, serialization dispatch |
| **_libs -> core** | **VIOLATION** | **20+** | Block construction, dtype detection |
| `io -> io` | INTERNAL | 95 | I/O subpackage cross-refs |
| `core -> core` | INTERNAL | 879 | Core subpackage cross-refs |

### Heaviest Internal Cross-Dependencies within `core/`

| Subpackage | Most Depended On By |
|------------|-------------------|
| `core/dtypes/` | arrays (73), indexes (43), reshape (27), internals (26), groupby (13), window (9) |
| `core/arrays/` | indexes (14), internals (7), reshape (9), groupby (6), tools (6) |
| `core/indexes/` | groupby (4), internals (3), reshape (7), window (2) |
| `core/_libs (via core)` | arrays (52), indexes (28), dtypes (21), tools (11), groupby (8) |

---

## Module Count Summary

| Category | Python Modules | Cython Modules | Total |
|----------|---------------|----------------|-------|
| `core/` | 173 | -- | 173 |
| `io/` | 56 | -- | 56 |
| `_libs/` | 3 | 40 | 43 |
| `plotting/` | 13 | -- | 13 |
| `_testing/` | 7 | -- | 7 |
| `util/` | 9 | -- | 9 |
| `compat/` | 7 | -- | 7 |
| `api/` | 9 | -- | 9 |
| `tseries/` | 5 | -- | 5 |
| `_config/` | 5 | -- | 5 |
| `errors/` | 2 | -- | 2 |
| `arrays/` | 1 | -- | 1 |
| Root modules | 3 | -- | 3 |
| **TOTAL** | **293** | **40** | **333** |

---

## FrankenPandas Rust Port Implications

### Layer Mapping to FrankenPandas Crates

| pandas Layer | FP Crate(s) | Status |
|-------------|-------------|--------|
| `_libs/` algorithms | `fp-algos` | Partial |
| `_libs/` hashtable | `fp-index` (hash engines) | Partial |
| `_libs/tslibs/` | Not yet started | Planned |
| `core/dtypes/` | `fp-common` (DType enum) | Partial |
| `core/arrays/` | `fp-column` | Partial |
| `core/indexes/` | `fp-index` | Partial |
| `core/internals/` | `fp-frame` (BlockManager equiv) | Partial |
| `core/frame.py` | `fp-frame` (DataFrame) | Partial |
| `core/series.py` | `fp-frame` (Series) | Partial |
| `core/groupby/` | `fp-groupby` | Partial |
| `core/reshape/merge.py` | `fp-join` | Partial |
| `core/ops/` | `fp-column` (ops) | Partial |
| `core/nanops.py` | `fp-column` (reductions) | Partial |
| `io/` | `fp-io` | Partial |
| `core/computation/` | `fp-expr` | Partial |
| `errors/` | `fp-common` (Error types) | Partial |

### Key Observations for Rust Port

1. **The `io.formats.printing` problem**: This module (597 LOC) is imported by core but lives in io. In FrankenPandas, formatting should live in `fp-common` or a dedicated `fp-format` crate, not in `fp-io`.

2. **`_libs -> core` circular dependency**: The Cython layer's upward imports to core (for block construction, dtype detection) mean these must be resolved in the Rust port. The Rust trait system can cleanly separate the interface (`fp-common` traits) from implementation (`fp-column`, `fp-frame`).

3. **`core/generic.py` (12,788 LOC) is a God Object**: NDFrame contains too much. The Rust port should decompose this into:
   - Alignment trait (AACE)
   - Indexing trait
   - Metadata management
   - Aggregation dispatch

4. **`core/frame.py` (18,679 LOC) is the largest file**: Many methods are I/O dispatch stubs (`.to_parquet()`, `.to_stata()`, etc.). In Rust, these should be implemented as trait impls in `fp-io`, not methods on DataFrame.

5. **The dtype system** (`core/dtypes/`, 9,102 LOC) is central to everything. FrankenPandas' `DType` enum approach is correct but must expand significantly to cover all the casting/coercion logic in `cast.py` (1,879 LOC) and `common.py` (1,988 LOC).

---

*Generated by DOC-PASS-01 cartography scan. Source: pandas 3.x (commit at legacy_pandas_code/pandas/).*
