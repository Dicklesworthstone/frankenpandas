# DOC-PASS-02: Symbol/API Census and Surface Classification

**Bead:** bd-2gi.23.3
**Date:** 2026-02-14
**Scope:** Comprehensive enumeration of the pandas public API surface, classification by stability/visibility/frequency, call-context mapping, and FrankenPandas Rust coverage status.

---

## Table of Contents

1. [Methodology](#1-methodology)
2. [Top-Level Public API](#2-top-level-public-api)
3. [DataFrame API Surface](#3-dataframe-api-surface)
4. [Series API Surface](#4-series-api-surface)
5. [Index Hierarchy API Surface](#5-index-hierarchy-api-surface)
6. [GroupBy API Surface](#6-groupby-api-surface)
7. [IO Functions](#7-io-functions)
8. [Reshaping & Merge Functions](#8-reshaping--merge-functions)
9. [Window Functions](#9-window-functions)
10. [Accessor APIs (str, dt, cat)](#10-accessor-apis-str-dt-cat)
11. [Type System & DType Classes](#11-type-system--dtype-classes)
12. [Configuration & Utility](#12-configuration--utility)
13. [Call-Context Maps](#13-call-context-maps)
14. [FrankenPandas Coverage Summary](#14-frankenpandas-coverage-summary)
15. [Coverage Gap Prioritization](#15-coverage-gap-prioritization)

---

## 1. Methodology

### Sources Analyzed
- `pandas/__init__.py` `__all__` export list (95 symbols)
- `pandas/core/frame.py` -- DataFrame class (~250 public methods/properties)
- `pandas/core/series.py` -- Series class (~200 public methods/properties)
- `pandas/core/generic.py` -- NDFrame shared base (~150 methods)
- `pandas/core/indexes/base.py` -- Index base class (~120 methods)
- `pandas/core/indexes/{range,multi,category,datetimes,timedeltas,period,interval}.py`
- `pandas/core/groupby/{groupby,generic}.py` -- GroupBy hierarchy (~80 methods)
- `pandas/core/window/{rolling,expanding,ewm}.py` -- Window functions (~40 methods)
- `pandas/core/strings/accessor.py` -- str accessor (~60 methods)
- `pandas/io/api.py` -- IO API (21 symbols)
- `pandas/core/reshape/api.py` -- Reshape API (14 symbols)
- `pandas/errors/__init__.py` -- Error classes

### FrankenPandas Crates Analyzed
- `fp-types` -- Scalar, DType, NA/NaN operations
- `fp-columnar` -- Column, ValidityMask, CrackIndex
- `fp-index` -- Index, AlignmentPlan, AlignMode, leapfrog operations
- `fp-frame` -- Series, DataFrame, concat
- `fp-groupby` -- groupby_*, AggFunc, sketches (HLL, KLL, CMS)
- `fp-join` -- join_series, merge_dataframes, JoinType
- `fp-io` -- CSV/JSON read/write
- `fp-expr` -- Expr, EvalContext, MaterializedView (IVM)
- `fp-runtime` -- RuntimePolicy, ConformalGuard, EvidenceLedger
- `fp-conformance` -- Test harness, parity gates, CI pipeline

### Classification Key

**Stability:**
- `STABLE` -- Part of the guaranteed API; backward-compatible
- `EXPERIMENTAL` -- May change between minor versions
- `DEPRECATED` -- Scheduled for removal; emits warnings

**Visibility:**
- `PUBLIC` -- Documented in `__all__`, intended for end users
- `SEMI-PUBLIC` -- Accessible but not in `__all__`; used in advanced patterns
- `INTERNAL` -- Underscore-prefixed or implementation detail

**Usage Frequency:**
- `CORE` -- Used daily by most pandas users
- `COMMON` -- Used regularly in typical workflows
- `NICHE` -- Specialized; used by <10% of users

**FP Coverage:**
- `FULL` -- FrankenPandas has a complete Rust implementation
- `PARTIAL` -- Some functionality exists but not all parameters/edge cases
- `STUB` -- API exists but minimal implementation
- `NONE` -- No FrankenPandas equivalent yet

---

## 2. Top-Level Public API

These are the 95 symbols exported from `pandas/__init__.__all__`.

### 2.1 Core Data Structures

| Symbol | Kind | Stability | Visibility | Frequency | FP Coverage | FP Crate |
|--------|------|-----------|------------|-----------|-------------|----------|
| `DataFrame` | class | STABLE | PUBLIC | CORE | PARTIAL | fp-frame |
| `Series` | class | STABLE | PUBLIC | CORE | PARTIAL | fp-frame |
| `Index` | class | STABLE | PUBLIC | CORE | PARTIAL | fp-index |
| `RangeIndex` | class | STABLE | PUBLIC | COMMON | PARTIAL | fp-index (`from_range`) |
| `MultiIndex` | class | STABLE | PUBLIC | COMMON | NONE | -- |
| `CategoricalIndex` | class | STABLE | PUBLIC | NICHE | NONE | -- |
| `DatetimeIndex` | class | STABLE | PUBLIC | COMMON | NONE | -- |
| `TimedeltaIndex` | class | STABLE | PUBLIC | NICHE | NONE | -- |
| `PeriodIndex` | class | STABLE | PUBLIC | NICHE | NONE | -- |
| `IntervalIndex` | class | STABLE | PUBLIC | NICHE | NONE | -- |
| `Categorical` | class | STABLE | PUBLIC | COMMON | NONE | -- |

### 2.2 Scalar Types & Sentinels

| Symbol | Kind | Stability | Visibility | Frequency | FP Coverage | FP Crate |
|--------|------|-----------|------------|-----------|-------------|----------|
| `NA` | sentinel | STABLE | PUBLIC | CORE | FULL | fp-types (`Scalar::Null`) |
| `NaT` | sentinel | STABLE | PUBLIC | COMMON | NONE | -- |
| `Timestamp` | class | STABLE | PUBLIC | CORE | NONE | -- |
| `Timedelta` | class | STABLE | PUBLIC | COMMON | NONE | -- |
| `Period` | class | STABLE | PUBLIC | NICHE | NONE | -- |
| `Interval` | class | STABLE | PUBLIC | NICHE | NONE | -- |
| `DateOffset` | class | STABLE | PUBLIC | COMMON | NONE | -- |

### 2.3 DType Classes

| Symbol | Kind | Stability | Visibility | Frequency | FP Coverage | FP Crate |
|--------|------|-----------|------------|-----------|-------------|----------|
| `Int8Dtype` | dtype | STABLE | PUBLIC | COMMON | PARTIAL | fp-types (`DType::Int64`) |
| `Int16Dtype` | dtype | STABLE | PUBLIC | COMMON | PARTIAL | fp-types |
| `Int32Dtype` | dtype | STABLE | PUBLIC | COMMON | PARTIAL | fp-types |
| `Int64Dtype` | dtype | STABLE | PUBLIC | CORE | FULL | fp-types (`DType::Int64`) |
| `UInt8Dtype` | dtype | STABLE | PUBLIC | NICHE | NONE | -- |
| `UInt16Dtype` | dtype | STABLE | PUBLIC | NICHE | NONE | -- |
| `UInt32Dtype` | dtype | STABLE | PUBLIC | NICHE | NONE | -- |
| `UInt64Dtype` | dtype | STABLE | PUBLIC | NICHE | NONE | -- |
| `Float32Dtype` | dtype | STABLE | PUBLIC | COMMON | PARTIAL | fp-types |
| `Float64Dtype` | dtype | STABLE | PUBLIC | CORE | FULL | fp-types (`DType::Float64`) |
| `BooleanDtype` | dtype | STABLE | PUBLIC | COMMON | FULL | fp-types (`DType::Bool`) |
| `StringDtype` | dtype | STABLE | PUBLIC | CORE | FULL | fp-types (`DType::Utf8`) |
| `CategoricalDtype` | dtype | STABLE | PUBLIC | COMMON | NONE | -- |
| `PeriodDtype` | dtype | STABLE | PUBLIC | NICHE | NONE | -- |
| `IntervalDtype` | dtype | STABLE | PUBLIC | NICHE | NONE | -- |
| `DatetimeTZDtype` | dtype | STABLE | PUBLIC | COMMON | NONE | -- |
| `SparseDtype` | dtype | STABLE | PUBLIC | NICHE | NONE | -- |
| `ArrowDtype` | dtype | STABLE | PUBLIC | COMMON | NONE | -- |

### 2.4 Missing Data Functions

| Symbol | Kind | Stability | Visibility | Frequency | FP Coverage | FP Crate |
|--------|------|-----------|------------|-----------|-------------|----------|
| `isna` | function | STABLE | PUBLIC | CORE | FULL | fp-types (`isna`) |
| `isnull` | function | STABLE | PUBLIC | CORE | FULL | fp-types (alias) |
| `notna` | function | STABLE | PUBLIC | CORE | FULL | fp-types (`notna`) |
| `notnull` | function | STABLE | PUBLIC | CORE | FULL | fp-types (alias) |

### 2.5 Conversion Functions

| Symbol | Kind | Stability | Visibility | Frequency | FP Coverage | FP Crate |
|--------|------|-----------|------------|-----------|-------------|----------|
| `to_numeric` | function | STABLE | PUBLIC | COMMON | NONE | -- |
| `to_datetime` | function | STABLE | PUBLIC | CORE | NONE | -- |
| `to_timedelta` | function | STABLE | PUBLIC | COMMON | NONE | -- |

### 2.6 Top-Level Utility Functions

| Symbol | Kind | Stability | Visibility | Frequency | FP Coverage | FP Crate |
|--------|------|-----------|------------|-----------|-------------|----------|
| `concat` | function | STABLE | PUBLIC | CORE | FULL | fp-frame (`concat_series`, `concat_dataframes`) |
| `merge` | function | STABLE | PUBLIC | CORE | FULL | fp-join (`merge_dataframes`) |
| `merge_asof` | function | STABLE | PUBLIC | COMMON | NONE | -- |
| `merge_ordered` | function | STABLE | PUBLIC | NICHE | NONE | -- |
| `cut` | function | STABLE | PUBLIC | COMMON | NONE | -- |
| `qcut` | function | STABLE | PUBLIC | COMMON | NONE | -- |
| `get_dummies` | function | STABLE | PUBLIC | COMMON | NONE | -- |
| `from_dummies` | function | STABLE | PUBLIC | NICHE | NONE | -- |
| `factorize` | function | STABLE | PUBLIC | COMMON | NONE | -- |
| `unique` | function | STABLE | PUBLIC | COMMON | PARTIAL | fp-index (`unique`) |
| `melt` | function | STABLE | PUBLIC | COMMON | NONE | -- |
| `pivot` | function | STABLE | PUBLIC | COMMON | NONE | -- |
| `pivot_table` | function | STABLE | PUBLIC | COMMON | NONE | -- |
| `crosstab` | function | STABLE | PUBLIC | COMMON | NONE | -- |
| `wide_to_long` | function | STABLE | PUBLIC | NICHE | NONE | -- |
| `lreshape` | function | STABLE | PUBLIC | NICHE | NONE | -- |
| `eval` | function | STABLE | PUBLIC | COMMON | PARTIAL | fp-expr (`evaluate`) |
| `date_range` | function | STABLE | PUBLIC | CORE | NONE | -- |
| `bdate_range` | function | STABLE | PUBLIC | NICHE | NONE | -- |
| `period_range` | function | STABLE | PUBLIC | NICHE | NONE | -- |
| `timedelta_range` | function | STABLE | PUBLIC | NICHE | NONE | -- |
| `interval_range` | function | STABLE | PUBLIC | NICHE | NONE | -- |
| `infer_freq` | function | STABLE | PUBLIC | NICHE | NONE | -- |
| `array` | function | STABLE | PUBLIC | COMMON | NONE | -- |
| `json_normalize` | function | STABLE | PUBLIC | COMMON | NONE | -- |
| `Grouper` | class | STABLE | PUBLIC | COMMON | NONE | -- |
| `NamedAgg` | namedtuple | STABLE | PUBLIC | COMMON | NONE | -- |
| `Flags` | class | STABLE | PUBLIC | NICHE | NONE | -- |
| `IndexSlice` | helper | STABLE | PUBLIC | COMMON | NONE | -- |
| `set_eng_float_format` | function | STABLE | PUBLIC | NICHE | NONE | -- |
| `show_versions` | function | STABLE | PUBLIC | NICHE | NONE | -- |
| `col` | function | EXPERIMENTAL | PUBLIC | NICHE | NONE | -- |
| `test` | function | STABLE | PUBLIC | NICHE | NONE | -- |

### 2.7 Configuration

| Symbol | Kind | Stability | Visibility | Frequency | FP Coverage | FP Crate |
|--------|------|-----------|------------|-----------|-------------|----------|
| `get_option` | function | STABLE | PUBLIC | COMMON | NONE | -- |
| `set_option` | function | STABLE | PUBLIC | COMMON | NONE | -- |
| `reset_option` | function | STABLE | PUBLIC | NICHE | NONE | -- |
| `describe_option` | function | STABLE | PUBLIC | NICHE | NONE | -- |
| `option_context` | context mgr | STABLE | PUBLIC | COMMON | NONE | -- |
| `options` | object | STABLE | PUBLIC | COMMON | NONE | -- |

### 2.8 Sub-Modules

| Symbol | Kind | Stability | Visibility | Frequency | FP Coverage |
|--------|------|-----------|------------|-----------|-------------|
| `api` | module | STABLE | PUBLIC | NICHE | NONE |
| `arrays` | module | STABLE | PUBLIC | NICHE | NONE |
| `errors` | module | STABLE | PUBLIC | COMMON | NONE |
| `io` | module | STABLE | PUBLIC | COMMON | PARTIAL |
| `plotting` | module | STABLE | PUBLIC | COMMON | NONE |
| `tseries` | module | STABLE | PUBLIC | NICHE | NONE |
| `offsets` | module | STABLE | PUBLIC | COMMON | NONE |
| `testing` | module | STABLE | PUBLIC | COMMON | PARTIAL |

---

## 3. DataFrame API Surface

Source files: `pandas/core/frame.py` (~18,600 lines), `pandas/core/generic.py` (~12,700 lines)

### 3.1 Construction

| Method/Property | Stability | Visibility | Frequency | FP Coverage | FP Function |
|----------------|-----------|------------|-----------|-------------|-------------|
| `__init__(data, index, columns, dtype, copy)` | STABLE | PUBLIC | CORE | PARTIAL | `DataFrame::new`, `from_dict` |
| `from_dict(data, orient, dtype, columns)` | STABLE | PUBLIC | COMMON | FULL | `DataFrame::from_dict` |
| `from_records(data, index, columns, ...)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `from_arrow(data)` | STABLE | PUBLIC | NICHE | NONE | -- |

### 3.2 Properties & Attributes

| Property | Stability | Visibility | Frequency | FP Coverage | FP Function |
|----------|-----------|------------|-----------|-------------|-------------|
| `index` | STABLE | PUBLIC | CORE | FULL | `DataFrame::index()` |
| `columns` | STABLE | PUBLIC | CORE | FULL | `DataFrame::column_names()` |
| `dtypes` | STABLE | PUBLIC | CORE | PARTIAL | per-column via `Column::dtype()` |
| `values` | STABLE | PUBLIC | CORE | NONE | -- |
| `axes` | STABLE | PUBLIC | COMMON | PARTIAL | `index()` + `column_names()` |
| `ndim` | STABLE | PUBLIC | COMMON | NONE | (always 2) |
| `size` | STABLE | PUBLIC | COMMON | PARTIAL | `len() * num_columns()` |
| `shape` | STABLE | PUBLIC | CORE | PARTIAL | `(len(), num_columns())` |
| `empty` | STABLE | PUBLIC | COMMON | FULL | `is_empty()` |
| `T` | STABLE | PUBLIC | COMMON | NONE | -- |
| `style` | STABLE | PUBLIC | COMMON | NONE | -- |

### 3.3 Indexing & Selection

| Method | Stability | Visibility | Frequency | FP Coverage | FP Function |
|--------|-----------|------------|-----------|-------------|-------------|
| `__getitem__(key)` | STABLE | PUBLIC | CORE | PARTIAL | `column()`, `select_columns()` |
| `__setitem__(key, value)` | STABLE | PUBLIC | CORE | PARTIAL | `with_column()` |
| `loc[...]` | STABLE | PUBLIC | CORE | NONE | -- |
| `iloc[...]` | STABLE | PUBLIC | CORE | NONE | -- |
| `at[...]` | STABLE | PUBLIC | COMMON | NONE | -- |
| `iat[...]` | STABLE | PUBLIC | COMMON | NONE | -- |
| `xs(key, axis, level)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `get(key, default)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `query(expr)` | STABLE | PUBLIC | CORE | NONE | -- |
| `eval(expr)` | STABLE | PUBLIC | COMMON | PARTIAL | fp-expr `evaluate()` |
| `select_dtypes(include, exclude)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `filter(items, like, regex, axis)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `head(n)` | STABLE | PUBLIC | CORE | FULL | `DataFrame::head()` |
| `tail(n)` | STABLE | PUBLIC | CORE | FULL | `DataFrame::tail()` |
| `take(indices)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `sample(n, frac, ...)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `isin(values)` | STABLE | PUBLIC | COMMON | PARTIAL | fp-index `isin()` |
| `where(cond, other)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `mask(cond, other)` | STABLE | PUBLIC | COMMON | NONE | -- |

### 3.4 Column Operations

| Method | Stability | Visibility | Frequency | FP Coverage | FP Function |
|--------|-----------|------------|-----------|-------------|-------------|
| `insert(loc, column, value)` | STABLE | PUBLIC | COMMON | PARTIAL | `with_column()` (append only) |
| `assign(**kwargs)` | STABLE | PUBLIC | CORE | NONE | -- |
| `pop(item)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `drop(labels, axis, columns)` | STABLE | PUBLIC | CORE | PARTIAL | `drop_column()` |
| `rename(mapper, columns)` | STABLE | PUBLIC | CORE | FULL | `rename_columns()` |
| `set_index(keys)` | STABLE | PUBLIC | CORE | NONE | -- |
| `reset_index()` | STABLE | PUBLIC | CORE | NONE | -- |
| `set_axis(labels, axis)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `reindex(labels)` | STABLE | PUBLIC | COMMON | PARTIAL | Series-level `reindex()` |
| `reindex_like(other)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `add_prefix(prefix)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `add_suffix(suffix)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `droplevel(level)` | STABLE | PUBLIC | NICHE | NONE | -- |

### 3.5 Missing Data

| Method | Stability | Visibility | Frequency | FP Coverage | FP Function |
|--------|-----------|------------|-----------|-------------|-------------|
| `isna()` / `isnull()` | STABLE | PUBLIC | CORE | PARTIAL | via column-level |
| `notna()` / `notnull()` | STABLE | PUBLIC | CORE | PARTIAL | via column-level |
| `dropna(axis, how, thresh, subset)` | STABLE | PUBLIC | CORE | PARTIAL | Series-level `dropna()` |
| `fillna(value, method, axis)` | STABLE | PUBLIC | CORE | PARTIAL | Series-level `fillna()` |
| `ffill(limit)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `bfill(limit)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `interpolate(method, ...)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `replace(to_replace, value)` | STABLE | PUBLIC | COMMON | NONE | -- |

### 3.6 Arithmetic & Comparison (element-wise)

| Method | Stability | Visibility | Frequency | FP Coverage | FP Function |
|--------|-----------|------------|-----------|-------------|-------------|
| `add(other)` / `__add__` | STABLE | PUBLIC | CORE | FULL | Series `add()`, Column `binary_numeric` |
| `sub(other)` / `__sub__` | STABLE | PUBLIC | CORE | FULL | Series `sub()` |
| `mul(other)` / `__mul__` | STABLE | PUBLIC | CORE | FULL | Series `mul()` |
| `truediv(other)` / `__truediv__` | STABLE | PUBLIC | CORE | FULL | Series `div()` |
| `floordiv(other)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `mod(other)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `pow(other)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `radd`, `rsub`, `rmul`, `rtruediv` | STABLE | PUBLIC | NICHE | NONE | -- |
| `rfloordiv`, `rmod`, `rpow` | STABLE | PUBLIC | NICHE | NONE | -- |
| `eq(other)` | STABLE | PUBLIC | CORE | FULL | Series `eq_series()`, Column `binary_comparison` |
| `ne(other)` | STABLE | PUBLIC | CORE | FULL | Series `ne_series()` |
| `lt(other)` | STABLE | PUBLIC | CORE | FULL | Series `lt()` |
| `le(other)` | STABLE | PUBLIC | CORE | FULL | Series `le()` |
| `gt(other)` | STABLE | PUBLIC | CORE | FULL | Series `gt()` |
| `ge(other)` | STABLE | PUBLIC | CORE | FULL | Series `ge()` |
| `compare(other)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `combine(other, func)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `combine_first(other)` | STABLE | PUBLIC | COMMON | FULL | Series `combine_first()` |
| `dot(other)` / `__matmul__` | STABLE | PUBLIC | NICHE | NONE | -- |
| `abs()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `clip(lower, upper)` | STABLE | PUBLIC | COMMON | NONE | -- |

### 3.7 Aggregation & Statistics

| Method | Stability | Visibility | Frequency | FP Coverage | FP Function |
|--------|-----------|------------|-----------|-------------|-------------|
| `count()` | STABLE | PUBLIC | CORE | FULL | Series `count()` |
| `sum()` | STABLE | PUBLIC | CORE | FULL | Series `sum()` |
| `mean()` | STABLE | PUBLIC | CORE | FULL | Series `mean()` |
| `median()` | STABLE | PUBLIC | CORE | FULL | Series `median()` |
| `min()` | STABLE | PUBLIC | CORE | FULL | Series `min()` |
| `max()` | STABLE | PUBLIC | CORE | FULL | Series `max()` |
| `std(ddof)` | STABLE | PUBLIC | CORE | FULL | Series `std()` |
| `var(ddof)` | STABLE | PUBLIC | CORE | FULL | Series `var()` |
| `sem()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `skew()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `kurt()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `prod()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `any()` | STABLE | PUBLIC | CORE | NONE | -- |
| `all()` | STABLE | PUBLIC | CORE | NONE | -- |
| `describe()` | STABLE | PUBLIC | CORE | NONE | -- |
| `nunique()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `value_counts()` | STABLE | PUBLIC | CORE | NONE | -- |
| `mode()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `quantile(q)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `rank()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `pct_change()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `corr()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `cov()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `corrwith(other)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `nlargest(n, columns)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `nsmallest(n, columns)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `cumsum()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `cumprod()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `cummin()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `cummax()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `diff(periods)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `idxmin()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `idxmax()` | STABLE | PUBLIC | COMMON | NONE | -- |

### 3.8 Sorting & Reordering

| Method | Stability | Visibility | Frequency | FP Coverage | FP Function |
|--------|-----------|------------|-----------|-------------|-------------|
| `sort_values(by)` | STABLE | PUBLIC | CORE | NONE | -- |
| `sort_index()` | STABLE | PUBLIC | CORE | NONE | -- |
| `nlargest(n, columns)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `nsmallest(n, columns)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `swaplevel(i, j)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `reorder_levels(order)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `transpose()` / `T` | STABLE | PUBLIC | COMMON | NONE | -- |

### 3.9 Reshaping

| Method | Stability | Visibility | Frequency | FP Coverage | FP Function |
|--------|-----------|------------|-----------|-------------|-------------|
| `pivot(columns, index, values)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `pivot_table(values, index, columns, aggfunc)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `melt(id_vars, value_vars)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `stack(level)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `unstack(level)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `explode(column)` | STABLE | PUBLIC | COMMON | NONE | -- |

### 3.10 Grouping, Joining & Merging

| Method | Stability | Visibility | Frequency | FP Coverage | FP Function |
|--------|-----------|------------|-----------|-------------|-------------|
| `groupby(by)` | STABLE | PUBLIC | CORE | PARTIAL | fp-groupby functions |
| `join(other, on, how)` | STABLE | PUBLIC | CORE | PARTIAL | fp-join `join_series` |
| `merge(right, on, how)` | STABLE | PUBLIC | CORE | FULL | fp-join `merge_dataframes` |
| `update(other)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `align(other, join)` | STABLE | PUBLIC | COMMON | FULL | fp-index `align()` |

### 3.11 Serialization & IO

| Method | Stability | Visibility | Frequency | FP Coverage | FP Function |
|--------|-----------|------------|-----------|-------------|-------------|
| `to_csv(path)` | STABLE | PUBLIC | CORE | FULL | fp-io `write_csv` |
| `to_json(path)` | STABLE | PUBLIC | CORE | FULL | fp-io `write_json` |
| `to_excel(path)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `to_parquet(path)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `to_feather(path)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `to_sql(name, con)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `to_hdf(path, key)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `to_pickle(path)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `to_html()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `to_xml(path)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `to_markdown()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `to_string()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `to_dict(orient)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `to_records()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `to_numpy()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `to_clipboard()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `to_stata(path)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `to_orc(path)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `to_iceberg(table)` | EXPERIMENTAL | PUBLIC | NICHE | NONE | -- |
| `to_latex()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `to_xarray()` | STABLE | PUBLIC | NICHE | NONE | -- |

### 3.12 Type Conversion

| Method | Stability | Visibility | Frequency | FP Coverage | FP Function |
|--------|-----------|------------|-----------|-------------|-------------|
| `astype(dtype)` | STABLE | PUBLIC | CORE | PARTIAL | fp-types `cast_scalar` |
| `copy(deep)` | STABLE | PUBLIC | CORE | NONE | -- |
| `infer_objects()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `convert_dtypes()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `to_timestamp()` | DEPRECATED | PUBLIC | NICHE | NONE | -- |
| `to_period()` | DEPRECATED | PUBLIC | NICHE | NONE | -- |

### 3.13 Functional Application

| Method | Stability | Visibility | Frequency | FP Coverage | FP Function |
|--------|-----------|------------|-----------|-------------|-------------|
| `apply(func, axis)` | STABLE | PUBLIC | CORE | NONE | -- |
| `map(func)` | STABLE | PUBLIC | CORE | NONE | -- |
| `transform(func)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `aggregate(func)` / `agg` | STABLE | PUBLIC | CORE | NONE | -- |
| `pipe(func)` | STABLE | PUBLIC | COMMON | NONE | -- |

### 3.14 Duplicate Handling

| Method | Stability | Visibility | Frequency | FP Coverage | FP Function |
|--------|-----------|------------|-----------|-------------|-------------|
| `drop_duplicates(subset, keep)` | STABLE | PUBLIC | CORE | NONE | -- |
| `duplicated(subset, keep)` | STABLE | PUBLIC | COMMON | NONE | -- |

### 3.15 Iteration

| Method | Stability | Visibility | Frequency | FP Coverage | FP Function |
|--------|-----------|------------|-----------|-------------|-------------|
| `items()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `iterrows()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `itertuples(index, name)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `keys()` | STABLE | PUBLIC | COMMON | PARTIAL | `column_names()` |
| `__iter__` | STABLE | PUBLIC | COMMON | NONE | -- |
| `__len__` | STABLE | PUBLIC | CORE | FULL | `len()` |

### 3.16 Window Operations

| Method | Stability | Visibility | Frequency | FP Coverage | FP Function |
|--------|-----------|------------|-----------|-------------|-------------|
| `rolling(window)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `expanding(min_periods)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `ewm(span, com, ...)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `resample(rule)` | STABLE | PUBLIC | COMMON | NONE | -- |

### 3.17 Miscellaneous

| Method | Stability | Visibility | Frequency | FP Coverage | FP Function |
|--------|-----------|------------|-----------|-------------|-------------|
| `info()` | STABLE | PUBLIC | CORE | NONE | -- |
| `memory_usage(deep)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `equals(other)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `squeeze()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `shift(periods)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `truncate(before, after)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `tz_convert(tz)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `tz_localize(tz)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `asof(where)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `at_time(time)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `between_time(start, end)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `asfreq(freq)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `first_valid_index()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `last_valid_index()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `set_flags(allows_duplicate_labels)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `__dataframe__()` | DEPRECATED | PUBLIC | NICHE | NONE | -- |
| `isetitem(loc, value)` | STABLE | SEMI-PUBLIC | NICHE | NONE | -- |

---

## 4. Series API Surface

Source file: `pandas/core/series.py` (~9,800 lines), inherits from NDFrame.

Most DataFrame methods in sections 3.5-3.8, 3.12-3.16 also apply to Series (inherited from NDFrame). Below lists Series-specific methods only.

### 4.1 Construction

| Method | Stability | Visibility | Frequency | FP Coverage | FP Function |
|--------|-----------|------------|-----------|-------------|-------------|
| `__init__(data, index, dtype, name)` | STABLE | PUBLIC | CORE | FULL | `Series::new`, `from_values`, `from_pairs` |
| `from_arrow(data)` | STABLE | PUBLIC | NICHE | NONE | -- |

### 4.2 Properties

| Property | Stability | Visibility | Frequency | FP Coverage | FP Function |
|----------|-----------|------------|-----------|-------------|-------------|
| `name` | STABLE | PUBLIC | CORE | FULL | `Series::name()` |
| `dtype` | STABLE | PUBLIC | CORE | PARTIAL | via `Column::dtype()` |
| `values` | STABLE | PUBLIC | CORE | FULL | `Series::values()` |
| `index` | STABLE | PUBLIC | CORE | FULL | `Series::index()` |
| `array` | STABLE | PUBLIC | COMMON | PARTIAL | `Series::column()` |
| `is_unique` | STABLE | PUBLIC | COMMON | PARTIAL | Index-level |
| `is_monotonic_increasing` | STABLE | PUBLIC | NICHE | PARTIAL | Index `is_sorted()` |
| `is_monotonic_decreasing` | STABLE | PUBLIC | NICHE | NONE | -- |
| `nbytes` | STABLE | PUBLIC | NICHE | NONE | -- |
| `hasnans` | STABLE | PUBLIC | COMMON | PARTIAL | fp-types `count_na` > 0 |

### 4.3 Series-Specific Methods

| Method | Stability | Visibility | Frequency | FP Coverage | FP Function |
|--------|-----------|------------|-----------|-------------|-------------|
| `unique()` | STABLE | PUBLIC | CORE | PARTIAL | fp-index `unique()` |
| `nunique()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `value_counts()` | STABLE | PUBLIC | CORE | NONE | -- |
| `mode()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `count()` | STABLE | PUBLIC | CORE | FULL | `Series::count()` |
| `idxmin()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `idxmax()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `round(decimals)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `quantile(q)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `corr(other)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `cov(other)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `diff(periods)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `autocorr(lag)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `searchsorted(value)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `repeat(repeats)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `between(left, right)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `case_when(caselist)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `isin(values)` | STABLE | PUBLIC | CORE | PARTIAL | fp-index `isin()` |
| `map(func)` | STABLE | PUBLIC | CORE | NONE | -- |
| `apply(func)` | STABLE | PUBLIC | CORE | NONE | -- |
| `to_frame(name)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `to_dict(into)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `to_list()` | STABLE | PUBLIC | COMMON | PARTIAL | `values()` returns slice |
| `explode()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `unstack(level)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `swaplevel(i, j)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `reorder_levels(order)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `argsort()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `nlargest(n)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `nsmallest(n)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `groupby(by)` | STABLE | PUBLIC | CORE | PARTIAL | fp-groupby |
| `update(other)` | STABLE | PUBLIC | NICHE | NONE | -- |

### 4.4 Series Arithmetic (all inherited + these overloads)

Full arithmetic set: `add`, `sub`, `mul`, `truediv`, `floordiv`, `mod`, `pow` plus `r*` reverses plus `divmod`, `rdivmod`. All comparison operators: `eq`, `ne`, `lt`, `le`, `gt`, `ge`.

| Category | Count | FP Coverage |
|----------|-------|-------------|
| Binary arithmetic (+, -, *, /) | 8 ops | FULL (4 basic), NONE (4 extended) |
| Reverse arithmetic | 7 ops | NONE |
| Comparison | 6 ops | FULL |
| Divmod | 2 ops | NONE |

### 4.5 Series Reduction/Aggregation

| Method | Stability | Visibility | Frequency | FP Coverage |
|--------|-----------|------------|-----------|-------------|
| `sum()` | STABLE | PUBLIC | CORE | FULL |
| `mean()` | STABLE | PUBLIC | CORE | FULL |
| `median()` | STABLE | PUBLIC | CORE | FULL |
| `min()` | STABLE | PUBLIC | CORE | FULL |
| `max()` | STABLE | PUBLIC | CORE | FULL |
| `std()` | STABLE | PUBLIC | CORE | FULL |
| `var()` | STABLE | PUBLIC | CORE | FULL |
| `prod()` | STABLE | PUBLIC | COMMON | NONE |
| `sem()` | STABLE | PUBLIC | NICHE | NONE |
| `skew()` | STABLE | PUBLIC | NICHE | NONE |
| `kurt()` | STABLE | PUBLIC | NICHE | NONE |
| `any()` | STABLE | PUBLIC | CORE | NONE |
| `all()` | STABLE | PUBLIC | CORE | NONE |
| `cumsum()` | STABLE | PUBLIC | COMMON | NONE |
| `cumprod()` | STABLE | PUBLIC | COMMON | NONE |
| `cummin()` | STABLE | PUBLIC | COMMON | NONE |
| `cummax()` | STABLE | PUBLIC | COMMON | NONE |

---

## 5. Index Hierarchy API Surface

### 5.1 Base Index Class

Source: `pandas/core/indexes/base.py` (~6,800 lines)

| Method/Property | Stability | Visibility | Frequency | FP Coverage | FP Function |
|----------------|-----------|------------|-----------|-------------|-------------|
| `__init__(data, dtype, name)` | STABLE | PUBLIC | CORE | FULL | `Index::new()` |
| `name` | STABLE | PUBLIC | CORE | NONE | -- (labels only) |
| `names` | STABLE | PUBLIC | COMMON | NONE | -- |
| `dtype` | STABLE | PUBLIC | CORE | NONE | -- |
| `values` | STABLE | PUBLIC | CORE | FULL | `Index::labels()` |
| `is_unique` | STABLE | PUBLIC | COMMON | FULL | `!has_duplicates()` |
| `has_duplicates` | STABLE | PUBLIC | COMMON | FULL | `has_duplicates()` (OnceCell) |
| `is_monotonic_increasing` | STABLE | PUBLIC | COMMON | FULL | `is_sorted()` |
| `is_monotonic_decreasing` | STABLE | PUBLIC | NICHE | NONE | -- |
| `inferred_type` | STABLE | PUBLIC | NICHE | NONE | -- |
| `hasnans` | STABLE | PUBLIC | COMMON | NONE | -- |
| `__len__` | STABLE | PUBLIC | CORE | FULL | `len()` |
| `__contains__(key)` | STABLE | PUBLIC | CORE | FULL | `contains()` |
| `__getitem__(key)` | STABLE | PUBLIC | CORE | PARTIAL | `slice()`, `take()` |
| `get_loc(key)` | STABLE | PUBLIC | CORE | FULL | `position()` |
| `get_indexer(target)` | STABLE | PUBLIC | COMMON | FULL | `get_indexer()` |
| `get_indexer_non_unique(target)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `unique()` | STABLE | PUBLIC | COMMON | FULL | `unique()` |
| `duplicated(keep)` | STABLE | PUBLIC | COMMON | FULL | `duplicated()` |
| `drop_duplicates(keep)` | STABLE | PUBLIC | COMMON | FULL | `drop_duplicates()` |
| `isin(values)` | STABLE | PUBLIC | COMMON | FULL | `isin()` |
| `union(other)` | STABLE | PUBLIC | COMMON | FULL | `union_with()` |
| `intersection(other)` | STABLE | PUBLIC | COMMON | FULL | `intersection()` |
| `difference(other)` | STABLE | PUBLIC | COMMON | FULL | `difference()` |
| `symmetric_difference(other)` | STABLE | PUBLIC | COMMON | FULL | `symmetric_difference()` |
| `sort_values(ascending)` | STABLE | PUBLIC | COMMON | FULL | `sort_values()` |
| `argsort()` | STABLE | PUBLIC | NICHE | FULL | `argsort()` |
| `take(indices)` | STABLE | PUBLIC | NICHE | FULL | `take()` |
| `copy()` | STABLE | PUBLIC | COMMON | NONE | -- (Clone impl) |
| `isna()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `notna()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `fillna(value)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `dropna(how)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `astype(dtype)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `map(mapper)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `rename(name)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `set_names(names)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `droplevel(level)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `to_series(index, name)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `to_frame(index, name)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `to_flat_index()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `join(other, how)` | STABLE | SEMI-PUBLIC | COMMON | FULL | fp-index `align()` |
| `reindex(target)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `equals(other)` | STABLE | PUBLIC | COMMON | PARTIAL | PartialEq impl |
| `identical(other)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `where(cond, other)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `append(other)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `memory_usage(deep)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `repeat(repeats)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `slice_indexer(start, end)` | STABLE | SEMI-PUBLIC | NICHE | PARTIAL | `slice()` |
| `shift(periods)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `sortlevel(ascending)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `asof(label)` | STABLE | PUBLIC | NICHE | NONE | -- |

### 5.2 Index Subclasses

| Class | Stability | Key Extra Methods | FP Coverage |
|-------|-----------|-------------------|-------------|
| `RangeIndex` | STABLE | `start`, `stop`, `step`, `from_range()` | PARTIAL (`from_range`) |
| `MultiIndex` | STABLE | `from_tuples`, `from_arrays`, `from_product`, `get_level_values`, `set_levels`, `set_codes`, `swaplevel`, `reorder_levels`, `to_flat_index` | NONE |
| `CategoricalIndex` | STABLE | `codes`, `categories`, `ordered`, `rename_categories`, `reorder_categories`, `add_categories`, `remove_categories`, `remove_unused_categories`, `set_categories` | NONE |
| `DatetimeIndex` | STABLE | `year`, `month`, `day`, `hour`, `minute`, `second`, `date`, `time`, `tz`, `freq`, `tz_convert`, `tz_localize`, `normalize`, `round`, `floor`, `ceil`, `to_period`, `to_pydatetime`, `snap`, `mean`, `std` | NONE |
| `TimedeltaIndex` | STABLE | `days`, `seconds`, `microseconds`, `nanoseconds`, `components`, `total_seconds`, `round`, `floor`, `ceil`, `mean` | NONE |
| `PeriodIndex` | STABLE | `year`, `month`, `day`, `hour`, `minute`, `second`, `freq`, `to_timestamp`, `asfreq` | NONE |
| `IntervalIndex` | STABLE | `left`, `right`, `mid`, `length`, `closed`, `is_overlapping`, `overlaps`, `set_closed`, `contains`, `from_breaks`, `from_tuples`, `from_arrays` | NONE |

### 5.3 FrankenPandas Alignment Engine (fp-index)

FrankenPandas has a rich alignment engine beyond what pandas Index exposes:

| FP Function | pandas Equivalent | Notes |
|-------------|-------------------|-------|
| `align(left, right, mode)` | `Index.join(how)`, `NDFrame.align()` | Produces AlignmentPlan with position arrays |
| `align_inner(left, right)` | `Index.join(how='inner')` | Optimized inner join path |
| `align_left(left, right)` | `Index.join(how='left')` | Left-join alignment |
| `align_union(left, right)` | `Index.join(how='outer')` | Outer-union with borrowed-key HashMap |
| `leapfrog_union(indexes)` | No direct equivalent | Multi-way union via leapfrog triejoin |
| `leapfrog_intersection(indexes)` | No direct equivalent | Multi-way intersection |
| `multi_way_align(indexes)` | No direct equivalent | N-way alignment plan |
| `validate_alignment_plan(plan)` | Internal | Plan validation |

---

## 6. GroupBy API Surface

Source: `pandas/core/groupby/groupby.py` (~5,900 lines), `pandas/core/groupby/generic.py` (~4,000 lines)

### 6.1 GroupBy Base Methods

| Method | Stability | Visibility | Frequency | FP Coverage | FP Function |
|--------|-----------|------------|-----------|-------------|-------------|
| `sum()` | STABLE | PUBLIC | CORE | FULL | `groupby_sum` |
| `mean()` | STABLE | PUBLIC | CORE | FULL | `groupby_mean` |
| `median()` | STABLE | PUBLIC | COMMON | FULL | `groupby_median` |
| `min()` | STABLE | PUBLIC | CORE | FULL | `groupby_min` |
| `max()` | STABLE | PUBLIC | CORE | FULL | `groupby_max` |
| `count()` | STABLE | PUBLIC | CORE | FULL | `groupby_count` |
| `std()` | STABLE | PUBLIC | COMMON | FULL | `groupby_std` |
| `var()` | STABLE | PUBLIC | COMMON | FULL | `groupby_var` |
| `first()` | STABLE | PUBLIC | COMMON | FULL | `groupby_first` |
| `last()` | STABLE | PUBLIC | COMMON | FULL | `groupby_last` |
| `sem()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `prod()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `size()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `any()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `all()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `ohlc()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `describe()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `apply(func)` | STABLE | PUBLIC | CORE | NONE | -- |
| `transform(func)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `aggregate(func)` / `agg` | STABLE | PUBLIC | CORE | PARTIAL | `groupby_agg` with `AggFunc` |
| `filter(func)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `pipe(func)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `get_group(name)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `nth()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `head(n)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `tail(n)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `ngroup()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `cumcount()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `cumsum()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `cumprod()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `cummin()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `cummax()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `rank()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `diff(periods)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `pct_change()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `shift(periods)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `ffill()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `bfill()` | STABLE | PUBLIC | COMMON | NONE | -- |
| `quantile(q)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `resample(rule)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `rolling(window)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `expanding()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `ewm()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `sample(n)` | STABLE | PUBLIC | NICHE | NONE | -- |

### 6.2 SeriesGroupBy-Specific

| Method | Stability | Visibility | Frequency | FP Coverage |
|--------|-----------|------------|-----------|-------------|
| `nunique()` | STABLE | PUBLIC | COMMON | NONE |
| `unique()` | STABLE | PUBLIC | COMMON | NONE |
| `value_counts()` | STABLE | PUBLIC | COMMON | NONE |
| `nlargest(n)` | STABLE | PUBLIC | NICHE | NONE |
| `nsmallest(n)` | STABLE | PUBLIC | NICHE | NONE |
| `idxmin()` | STABLE | PUBLIC | COMMON | NONE |
| `idxmax()` | STABLE | PUBLIC | COMMON | NONE |
| `corr()` | STABLE | PUBLIC | NICHE | NONE |
| `cov()` | STABLE | PUBLIC | NICHE | NONE |
| `skew()` | STABLE | PUBLIC | NICHE | NONE |
| `kurt()` | STABLE | PUBLIC | NICHE | NONE |
| `is_monotonic_increasing` | STABLE | PUBLIC | NICHE | NONE |
| `is_monotonic_decreasing` | STABLE | PUBLIC | NICHE | NONE |
| `dtype` | STABLE | PUBLIC | NICHE | NONE |

### 6.3 DataFrameGroupBy-Specific

| Method | Stability | Visibility | Frequency | FP Coverage |
|--------|-----------|------------|-----------|-------------|
| `nunique()` | STABLE | PUBLIC | COMMON | NONE |
| `value_counts()` | STABLE | PUBLIC | COMMON | NONE |
| `idxmin()` | STABLE | PUBLIC | COMMON | NONE |
| `idxmax()` | STABLE | PUBLIC | COMMON | NONE |
| `corr()` | STABLE | PUBLIC | NICHE | NONE |
| `cov()` | STABLE | PUBLIC | NICHE | NONE |
| `corrwith(other)` | STABLE | PUBLIC | NICHE | NONE |
| `skew()` | STABLE | PUBLIC | NICHE | NONE |
| `kurt()` | STABLE | PUBLIC | NICHE | NONE |
| `take(indices)` | STABLE | PUBLIC | NICHE | NONE |

### 6.4 FrankenPandas GroupBy Extras (not in pandas)

| FP Function | Category | Notes |
|-------------|----------|-------|
| `groupby_sum_with_options` | Performance | Execution options: dense bucket path, identity fast-path |
| `GroupByExecutionOptions` | Config | `force_dense_bucket`, `identity_fast_path` |
| `approx_nunique(values)` | Sketch | HyperLogLog approximate distinct count |
| `approx_quantile(values, q)` | Sketch | KLL sketch approximate quantile |
| `approx_value_counts(values)` | Sketch | Count-Min Sketch frequency estimation |
| `HyperLogLog` | Struct | Configurable precision HLL |
| `KllSketch` | Struct | Configurable accuracy KLL |
| `CountMinSketch` | Struct | Configurable width/depth CMS |

---

## 7. IO Functions

### 7.1 pandas IO API

Source: `pandas/io/api.py` (21 symbols)

| Function | Stability | Visibility | Frequency | FP Coverage | FP Function |
|----------|-----------|------------|-----------|-------------|-------------|
| `read_csv(filepath)` | STABLE | PUBLIC | CORE | FULL | fp-io `read_csv`, `read_csv_with_options` |
| `read_table(filepath)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `read_fwf(filepath)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `read_json(path)` | STABLE | PUBLIC | CORE | FULL | fp-io `read_json` |
| `read_excel(io)` | STABLE | PUBLIC | CORE | NONE | -- |
| `read_parquet(path)` | STABLE | PUBLIC | CORE | NONE | -- |
| `read_feather(path)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `read_orc(path)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `read_sql(sql, con)` | STABLE | PUBLIC | CORE | NONE | -- |
| `read_sql_query(sql, con)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `read_sql_table(table, con)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `read_html(io)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `read_xml(path)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `read_hdf(path, key)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `read_pickle(filepath)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `read_stata(filepath)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `read_sas(filepath)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `read_spss(path)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `read_clipboard()` | STABLE | PUBLIC | NICHE | NONE | -- |
| `read_iceberg(table)` | EXPERIMENTAL | PUBLIC | NICHE | NONE | -- |
| `to_pickle(obj, filepath)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `ExcelFile` | class | STABLE | PUBLIC | COMMON | NONE | -- |
| `ExcelWriter` | class | STABLE | PUBLIC | COMMON | NONE | -- |
| `HDFStore` | class | STABLE | PUBLIC | NICHE | NONE | -- |

### 7.2 FrankenPandas IO Extras

| FP Function | pandas Equivalent | Notes |
|-------------|-------------------|-------|
| `read_csv_str(input)` | `read_csv(StringIO(input))` | In-memory CSV parsing |
| `write_csv_string(frame)` | `df.to_csv()` returns string | In-memory CSV writing |
| `read_csv_with_options(input, opts)` | `read_csv` with kwargs | `CsvReadOptions`: delimiter, has_header, skip_rows, max_rows, na_values, column_names |
| `read_json_str(input, orient)` | `read_json(StringIO(input))` | In-memory JSON parsing |
| `write_json_string(frame, orient)` | `df.to_json()` returns string | In-memory JSON writing |
| `read_json(path, orient)` | `read_json(path)` | File-based JSON read |
| `write_json(frame, path, orient)` | `df.to_json(path)` | File-based JSON write |
| `JsonOrient` | `orient` param | `Records`, `Columns` |

---

## 8. Reshaping & Merge Functions

Source: `pandas/core/reshape/api.py` (14 symbols)

| Function | Stability | Visibility | Frequency | FP Coverage | FP Function |
|----------|-----------|------------|-----------|-------------|-------------|
| `concat(objs, axis, join)` | STABLE | PUBLIC | CORE | FULL | fp-frame `concat_series`, `concat_dataframes` |
| `merge(left, right, on, how)` | STABLE | PUBLIC | CORE | FULL | fp-join `merge_dataframes` |
| `merge_asof(left, right)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `merge_ordered(left, right)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `melt(frame, id_vars, value_vars)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `pivot(data, columns, index, values)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `pivot_table(data, values, index, columns)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `crosstab(index, columns)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `get_dummies(data, columns)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `from_dummies(data)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `cut(x, bins)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `qcut(x, q)` | STABLE | PUBLIC | COMMON | NONE | -- |
| `wide_to_long(df, stubnames, i, j)` | STABLE | PUBLIC | NICHE | NONE | -- |
| `lreshape(data, groups)` | STABLE | PUBLIC | NICHE | NONE | -- |

### FrankenPandas Join Engine (fp-join)

| FP Function | pandas Equivalent | Notes |
|-------------|-------------------|-------|
| `join_series(left, right, how)` | `Series.align()` | Inner/Left/Right/Outer with arena allocation |
| `join_series_with_options(left, right, how, opts)` | -- | `JoinExecutionOptions`: arena size, suffix config |
| `merge_dataframes(left, right, on, how)` | `pd.merge()` | Multi-column key merge |
| `JoinType::{Inner,Left,Right,Outer}` | `how` parameter | Typed enum |

---

## 9. Window Functions

### 9.1 Rolling

| Method | Stability | Visibility | Frequency | FP Coverage |
|--------|-----------|------------|-----------|-------------|
| `sum()` | STABLE | PUBLIC | COMMON | NONE |
| `mean()` | STABLE | PUBLIC | COMMON | NONE |
| `std(ddof)` | STABLE | PUBLIC | COMMON | NONE |
| `var(ddof)` | STABLE | PUBLIC | COMMON | NONE |
| `min()` | STABLE | PUBLIC | COMMON | NONE |
| `max()` | STABLE | PUBLIC | COMMON | NONE |
| `median()` | STABLE | PUBLIC | COMMON | NONE |
| `count()` | STABLE | PUBLIC | COMMON | NONE |
| `skew()` | STABLE | PUBLIC | NICHE | NONE |
| `kurt()` | STABLE | PUBLIC | NICHE | NONE |
| `sem(ddof)` | STABLE | PUBLIC | NICHE | NONE |
| `quantile(q)` | STABLE | PUBLIC | NICHE | NONE |
| `rank()` | STABLE | PUBLIC | NICHE | NONE |
| `apply(func)` | STABLE | PUBLIC | COMMON | NONE |
| `corr(other)` | STABLE | PUBLIC | NICHE | NONE |
| `cov(other)` | STABLE | PUBLIC | NICHE | NONE |
| `aggregate(func)` | STABLE | PUBLIC | COMMON | NONE |
| `pipe(func)` | STABLE | PUBLIC | NICHE | NONE |

### 9.2 Expanding

Same method set as Rolling (sum, mean, std, var, min, max, median, count, skew, kurt, sem, quantile, rank, corr, cov, aggregate, apply, pipe) plus `first()`, `last()`, `nunique()`.

**FP Coverage: NONE** for all window operations.

### 9.3 EWM (Exponentially Weighted)

| Method | Stability | Visibility | Frequency | FP Coverage |
|--------|-----------|------------|-----------|-------------|
| `mean()` | STABLE | PUBLIC | COMMON | NONE |
| `sum()` | STABLE | PUBLIC | NICHE | NONE |
| `std(bias)` | STABLE | PUBLIC | COMMON | NONE |
| `var(bias)` | STABLE | PUBLIC | COMMON | NONE |
| `corr(other)` | STABLE | PUBLIC | NICHE | NONE |
| `cov(other)` | STABLE | PUBLIC | NICHE | NONE |

---

## 10. Accessor APIs (str, dt, cat)

### 10.1 String Accessor (`Series.str`)

Source: `pandas/core/strings/accessor.py` (~60 methods)

**FP Coverage: NONE** for all string accessor methods.

| Method | Stability | Frequency |
|--------|-----------|-----------|
| `str.lower()` | STABLE | CORE |
| `str.upper()` | STABLE | CORE |
| `str.strip()`, `lstrip()`, `rstrip()` | STABLE | CORE |
| `str.contains(pat)` | STABLE | CORE |
| `str.startswith(pat)` | STABLE | CORE |
| `str.endswith(pat)` | STABLE | CORE |
| `str.replace(pat, repl)` | STABLE | CORE |
| `str.split(pat)` | STABLE | CORE |
| `str.join(sep)` | STABLE | COMMON |
| `str.cat(others, sep)` | STABLE | COMMON |
| `str.get(i)` | STABLE | COMMON |
| `str.slice(start, stop)` | STABLE | COMMON |
| `str.len()` | STABLE | COMMON |
| `str.find(sub)` / `rfind(sub)` | STABLE | COMMON |
| `str.match(pat)` / `fullmatch(pat)` | STABLE | COMMON |
| `str.extract(pat)` | STABLE | COMMON |
| `str.extractall(pat)` | STABLE | NICHE |
| `str.findall(pat)` | STABLE | COMMON |
| `str.count(pat)` | STABLE | COMMON |
| `str.pad(width)` / `center(width)` | STABLE | NICHE |
| `str.ljust(width)` / `rjust(width)` | STABLE | NICHE |
| `str.zfill(width)` | STABLE | NICHE |
| `str.wrap(width)` | STABLE | NICHE |
| `str.title()` | STABLE | NICHE |
| `str.capitalize()` | STABLE | NICHE |
| `str.swapcase()` | STABLE | NICHE |
| `str.casefold()` | STABLE | NICHE |
| `str.normalize(form)` | STABLE | NICHE |
| `str.isalnum()` through `str.isspace()` | STABLE | NICHE |
| `str.encode()` / `decode()` | STABLE | NICHE |
| `str.get_dummies(sep)` | STABLE | NICHE |
| `str.translate(table)` | STABLE | NICHE |
| `str.repeat(repeats)` | STABLE | NICHE |
| `str.slice_replace()` | STABLE | NICHE |
| `str.removeprefix(prefix)` | STABLE | NICHE |
| `str.removesuffix(suffix)` | STABLE | NICHE |
| `str.partition(sep)` / `rpartition()` | STABLE | NICHE |
| `str.rsplit(pat)` | STABLE | COMMON |
| `str.index(sub)` / `rindex(sub)` | STABLE | NICHE |

### 10.2 DateTime Accessor (`Series.dt`)

**FP Coverage: NONE** for all datetime accessor methods.

Core properties: `year`, `month`, `day`, `hour`, `minute`, `second`, `microsecond`, `nanosecond`, `date`, `time`, `timetz`, `dayofweek`, `day_of_week`, `dayofyear`, `day_of_year`, `quarter`, `is_month_start`, `is_month_end`, `is_quarter_start`, `is_quarter_end`, `is_year_start`, `is_year_end`, `is_leap_year`, `days_in_month`, `tz`, `freq`.

Core methods: `normalize()`, `strftime()`, `round()`, `floor()`, `ceil()`, `tz_localize()`, `tz_convert()`, `to_period()`, `to_pydatetime()`, `to_pytimedelta()`, `total_seconds()`.

### 10.3 Categorical Accessor (`Series.cat`)

**FP Coverage: NONE** for all categorical accessor methods.

Properties: `categories`, `ordered`, `codes`.
Methods: `rename_categories()`, `reorder_categories()`, `add_categories()`, `remove_categories()`, `remove_unused_categories()`, `set_categories()`, `as_ordered()`, `as_unordered()`.

### 10.4 Sparse Accessor (`Series.sparse`)

**FP Coverage: NONE**. Properties: `npoints`, `density`, `fill_value`, `sp_values`. Methods: `to_coo()`, `to_dense()`, `from_coo()`.

---

## 11. Type System & DType Classes

### 11.1 pandas Type System

| Class | Stability | Visibility | FP Coverage |
|-------|-----------|------------|-------------|
| `Int8Dtype` through `Int64Dtype` | STABLE | PUBLIC | PARTIAL (Int64 mapped) |
| `UInt8Dtype` through `UInt64Dtype` | STABLE | PUBLIC | NONE |
| `Float32Dtype`, `Float64Dtype` | STABLE | PUBLIC | PARTIAL/FULL |
| `BooleanDtype` | STABLE | PUBLIC | FULL |
| `StringDtype` | STABLE | PUBLIC | FULL |
| `CategoricalDtype` | STABLE | PUBLIC | NONE |
| `DatetimeTZDtype` | STABLE | PUBLIC | NONE |
| `PeriodDtype` | STABLE | PUBLIC | NONE |
| `IntervalDtype` | STABLE | PUBLIC | NONE |
| `SparseDtype` | STABLE | PUBLIC | NONE |
| `ArrowDtype` | STABLE | PUBLIC | NONE |

### 11.2 FrankenPandas Type System (fp-types)

| FP Type | pandas Equivalent | Status |
|---------|-------------------|--------|
| `DType::Int64` | `Int64Dtype` / `int64` | FULL |
| `DType::Float64` | `Float64Dtype` / `float64` | FULL |
| `DType::Bool` | `BooleanDtype` / `bool` | FULL |
| `DType::Utf8` | `StringDtype` / `object` (str) | FULL |
| `Scalar::Int64(i64)` | int/np.int64 | FULL |
| `Scalar::Float64(f64)` | float/np.float64 | FULL |
| `Scalar::Bool(bool)` | bool/np.bool_ | FULL |
| `Scalar::Utf8(String)` | str | FULL |
| `Scalar::Null(NullKind)` | `pd.NA`, `np.nan`, `pd.NaT` | FULL |
| `NullKind::NA` | `pd.NA` | FULL |
| `NullKind::NaN` | `np.nan` | FULL |
| `NullKind::NaT` | `pd.NaT` | STUB (kind only) |

Utility functions: `isna`, `notna`, `count_na`, `fill_na`, `dropna`, `common_dtype`, `infer_dtype`, `cast_scalar`, `cast_scalar_owned`, `semantic_eq`.

NaN-safe operations: `nansum`, `nanmean`, `nancount`, `nanmin`, `nanmax`, `nanmedian`, `nanvar`, `nanstd`.

---

## 12. Configuration & Utility

### 12.1 pandas Configuration

| Symbol | Stability | Visibility | Frequency | FP Coverage |
|--------|-----------|------------|-----------|-------------|
| `get_option(key)` | STABLE | PUBLIC | COMMON | NONE |
| `set_option(key, value)` | STABLE | PUBLIC | COMMON | NONE |
| `reset_option(key)` | STABLE | PUBLIC | NICHE | NONE |
| `describe_option(key)` | STABLE | PUBLIC | NICHE | NONE |
| `option_context(key, value)` | STABLE | PUBLIC | COMMON | NONE |
| `options` | STABLE | PUBLIC | COMMON | NONE |

### 12.2 FrankenPandas Runtime System (fp-runtime)

No pandas equivalent. This is FrankenPandas-specific infrastructure:

| FP Component | Purpose |
|-------------|---------|
| `RuntimePolicy` | Strict/Hardened mode configuration |
| `ConformalGuard` | Conformal prediction monitoring |
| `EvidenceLedger` | Decision audit trail |
| `GalaxyBrainCard` | Decision record rendering |
| `DecisionRecord` | Individual decision with loss matrix |

### 12.3 FrankenPandas Conformance System (fp-conformance)

No pandas equivalent. Test oracle infrastructure:

| FP Component | Purpose |
|-------------|---------|
| `HarnessConfig` / `HarnessReport` | Smoke test configuration |
| `PacketFixture` / `PacketParityReport` | Parity gate testing |
| `DifferentialResult` / `DriftRecord` | Behavioral drift detection |
| `CiGate` / `CiPipelineResult` | CI gate enforcement |
| `E2eConfig` / `E2eReport` | End-to-end validation |
| `ForensicLog` / `FailureForensicsReport` | Failure analysis |
| `RaptorQEnvelope` | Erasure coding for artifacts |

---

## 13. Call-Context Maps

### 13.1 Data Loading Pipeline
```
read_csv/read_json/read_excel -> DataFrame
  -> df.dtypes / df.info() / df.describe()           [inspection]
  -> df.head() / df.tail() / df.shape                 [quick look]
  -> df.columns / df.index                             [structure]
```
**FP Coverage: PARTIAL** (CSV/JSON read + head/tail/shape/columns/index)

### 13.2 Data Cleaning Pipeline
```
DataFrame
  -> df.isna().sum() / df.dropna() / df.fillna()      [missing data]
  -> df.drop_duplicates()                              [dedup]
  -> df.astype(dtype)                                  [type conversion]
  -> df.rename(columns=mapping)                        [rename]
  -> df.drop(columns=[...])                            [column removal]
  -> df.replace(to_replace, value)                     [value replacement]
```
**FP Coverage: PARTIAL** (isna/dropna/fillna/rename/drop at Series level; no replace)

### 13.3 Selection & Filtering Pipeline
```
DataFrame
  -> df[column_name]                                   [column select]
  -> df[df['col'] > value]                             [boolean mask]
  -> df.loc[rows, cols] / df.iloc[rows, cols]         [label/positional]
  -> df.query('expr')                                  [query expression]
  -> df.isin(values)                                   [membership test]
  -> df.between(left, right)                           [range filter]
```
**FP Coverage: PARTIAL** (column select + boolean filter + compare_scalar; no loc/iloc/query)

### 13.4 Aggregation Pipeline
```
DataFrame
  -> df.groupby('key')                                 [grouping]
    -> .sum() / .mean() / .count() / .agg(...)        [aggregation]
    -> .transform(func)                                [group transform]
    -> .filter(func)                                   [group filter]
  -> df.sum() / df.mean() / df.describe()             [global agg]
  -> df.value_counts()                                 [frequency]
```
**FP Coverage: PARTIAL** (groupby sum/mean/count/min/max/std/var/first/last/median; no transform/filter)

### 13.5 Join & Merge Pipeline
```
pd.merge(left, right, on='key', how='inner')           [merge]
left.join(right, how='left')                           [join]
pd.concat([df1, df2], axis=0)                          [vertical concat]
pd.concat([df1, df2], axis=1)                          [horizontal concat]
```
**FP Coverage: FULL** (merge_dataframes, join_series, concat_series/dataframes -- all 4 join types)

### 13.6 Reshaping Pipeline
```
DataFrame
  -> df.pivot_table(values, index, columns, aggfunc)   [pivot]
  -> df.melt(id_vars, value_vars)                      [unpivot]
  -> df.stack() / df.unstack()                         [reshape]
  -> pd.get_dummies(df, columns)                       [encoding]
```
**FP Coverage: NONE**

### 13.7 Time Series Pipeline
```
pd.date_range(start, end, freq)                        [date generation]
df.resample('D').mean()                                [resampling]
df.rolling(window=7).mean()                            [rolling window]
df.ewm(span=7).mean()                                 [exponential smoothing]
df.shift(1) / df.diff(1) / df.pct_change()           [lag operations]
```
**FP Coverage: NONE**

### 13.8 IO Export Pipeline
```
DataFrame
  -> df.to_csv('file.csv')                             [CSV export]
  -> df.to_json('file.json')                           [JSON export]
  -> df.to_parquet('file.parquet')                     [Parquet export]
  -> df.to_excel('file.xlsx')                          [Excel export]
  -> df.to_sql('table', engine)                        [SQL export]
```
**FP Coverage: PARTIAL** (CSV + JSON; no Parquet/Excel/SQL)

---

## 14. FrankenPandas Coverage Summary

### 14.1 Quantitative Coverage by Domain

| Domain | pandas Symbols | FP Covered (FULL/PARTIAL) | Coverage % |
|--------|---------------|--------------------------|------------|
| **Top-level API** | 95 | 18 | 19% |
| **Type system** | 18 dtypes | 5 (Int64, Float64, Bool, Utf8, Null) | 28% |
| **DataFrame construction** | 4 | 2 | 50% |
| **DataFrame properties** | 11 | 7 | 64% |
| **DataFrame indexing/selection** | 19 | 6 | 32% |
| **DataFrame missing data** | 8 | 4 | 50% |
| **DataFrame arithmetic** | 22 ops | 10 | 45% |
| **DataFrame aggregation** | 30 | 8 | 27% |
| **DataFrame sorting** | 7 | 0 | 0% |
| **DataFrame reshaping** | 6 | 0 | 0% |
| **DataFrame groupby/join** | 5 | 4 | 80% |
| **DataFrame IO (export)** | 20 | 2 | 10% |
| **Series construction** | 2 | 1 | 50% |
| **Series properties** | 10 | 6 | 60% |
| **Series methods** | 35 | 6 | 17% |
| **Series aggregation** | 17 | 7 | 41% |
| **Index base** | 50 | 22 | 44% |
| **Index subclasses** | 7 classes | 1 (RangeIndex partial) | 14% |
| **GroupBy base** | 43 | 10 | 23% |
| **GroupBy Series-specific** | 14 | 0 | 0% |
| **GroupBy DataFrame-specific** | 10 | 0 | 0% |
| **IO read functions** | 21 | 2 | 10% |
| **Reshape functions** | 14 | 2 | 14% |
| **Window functions** | ~50 | 0 | 0% |
| **str accessor** | ~60 | 0 | 0% |
| **dt accessor** | ~35 | 0 | 0% |
| **cat accessor** | ~10 | 0 | 0% |
| **Configuration** | 6 | 0 | 0% |

### 14.2 Coverage by Frequency Tier

| Tier | pandas APIs | FP Covered | Coverage % |
|------|------------|------------|------------|
| **CORE** (daily use) | ~85 | ~40 | **47%** |
| **COMMON** (regular use) | ~180 | ~20 | **11%** |
| **NICHE** (specialized) | ~280 | ~5 | **2%** |
| **TOTAL** | ~545 | ~65 | **12%** |

### 14.3 Strongest Coverage Areas

1. **Index alignment & set operations** (44%) -- The AACE engine is well-covered with inner/left/outer alignment, plus leapfrog triejoin (unique to FP).
2. **Core arithmetic** (45%) -- The 4 basic operations plus all 6 comparisons on Series.
3. **Basic aggregation** (sum/mean/median/min/max/std/var/count) -- Covered at Series level AND groupby level.
4. **Join/merge** (80%) -- All 4 join types with arena allocation.
5. **CSV/JSON IO** (complete for these formats) -- Including options, string-based, and file-based.
6. **Missing data primitives** (isna/notna/fillna/dropna) -- At types, column, and series levels.

### 14.4 Weakest Coverage Areas (Gaps)

1. **Window functions** (0%) -- Rolling, expanding, EWM are all missing.
2. **String accessor** (0%) -- All ~60 str methods missing.
3. **DateTime/Timeseries** (0%) -- No datetime types, no dt accessor, no date_range, no resample.
4. **Reshaping** (0%) -- No pivot/melt/stack/unstack/get_dummies.
5. **loc/iloc indexing** (0%) -- No label-based or positional indexing API.
6. **Sorting** (0%) -- No sort_values/sort_index at DataFrame level.
7. **apply/map/transform** (0%) -- No functional application API.
8. **MultiIndex** (0%) -- No hierarchical indexing.
9. **Categorical** (0%) -- No categorical dtype or accessor.
10. **Parquet/Excel/SQL IO** (0%) -- Only CSV and JSON supported.

---

## 15. Coverage Gap Prioritization

### Priority 1: Core User Workflow Blockers

These gaps block the most common user workflows and should be addressed first:

| Gap | Impact | Blocked Workflows | Estimated Effort |
|-----|--------|-------------------|------------------|
| `sort_values` / `sort_index` | HIGH | Nearly all analysis | Medium |
| `loc` / `iloc` indexing | HIGH | Selection, mutation | High |
| `apply` / `map` | HIGH | Custom transforms | Medium |
| `value_counts` | HIGH | EDA, frequency analysis | Low |
| `describe` | HIGH | EDA | Medium |
| `any` / `all` | HIGH | Boolean reduction | Low |
| `assign` | HIGH | Method chaining | Low |
| `set_index` / `reset_index` | HIGH | Index management | Medium |

### Priority 2: Common Workflow Enablers

| Gap | Impact | Blocked Workflows | Estimated Effort |
|-----|--------|-------------------|------------------|
| `to_parquet` / `read_parquet` | MEDIUM | Modern data storage | Medium (Arrow dep) |
| `pivot_table` / `melt` | MEDIUM | Reshaping | High |
| `rolling().mean()` etc. | MEDIUM | Time series analysis | Medium |
| `str.contains` / `str.replace` | MEDIUM | Text processing | Medium |
| `groupby().transform()` | MEDIUM | Group-level transforms | Medium |
| `read_excel` | MEDIUM | Business data loading | Medium |
| `copy()` | MEDIUM | Defensive programming | Low (Clone) |
| `nunique` | MEDIUM | Cardinality analysis | Low |

### Priority 3: Ecosystem Completeness

| Gap | Impact | Notes |
|-----|--------|-------|
| DatetimeIndex / Timestamp | HIGH long-term | Requires new crate |
| MultiIndex | MEDIUM | Complex hierarchical indexing |
| CategoricalDtype | LOW | Niche but important for memory |
| Parquet/Arrow integration | MEDIUM | Industry standard format |
| SQL IO | MEDIUM | Database workflows |

### Priority Matrix (Impact vs Effort)

```
                    LOW EFFORT          MEDIUM EFFORT       HIGH EFFORT
HIGH IMPACT    | value_counts      | sort_values       | loc/iloc
               | any/all           | apply/map         | MultiIndex
               | assign            | set/reset_index   |
               | copy (Clone)      | describe          |
               |                   |                   |
MEDIUM IMPACT  | nunique           | rolling window    | pivot_table/melt
               | prod              | str accessor      | DatetimeIndex
               |                   | read_parquet      | read_excel
               |                   | groupby transform |
               |                   |                   |
LOW IMPACT     | sem/skew/kurt     | EWM window        | Categorical
               | nlargest/smallest | read_html/xml     | Sparse
               | swaplevel         | IntervalIndex     |
```

---

## Appendix A: Deprecated APIs Found in pandas 3.x

| API | Status | Replacement |
|-----|--------|-------------|
| `DataFrame.__dataframe__()` | DEPRECATED 3.0 | Use `pd.api.interchange.from_dataframe()` |
| `DataFrame.to_timestamp()` | DEPRECATED 3.0 | Use `PeriodIndex.to_timestamp()` |
| `DataFrame.to_period()` | DEPRECATED 3.0 | Use `DatetimeIndex.to_period()` |
| `stack(future_stack=False)` | DEPRECATED | New stack implementation |
| `DataFrame.applymap()` | DEPRECATED | Renamed to `DataFrame.map()` |
| `copy` parameter on many methods | DEPRECATED 3.0 | Copy-on-Write is default |
| `DataFrame.sum(axis=None)` behavior | DEPRECATED | Axis=None behavior changing |
| `DataFrame.prod(axis=None)` behavior | DEPRECATED | Axis=None behavior changing |
| `DataFrame.std/var/sem(axis=None)` behavior | DEPRECATED | Axis=None behavior changing |

**FrankenPandas strategy:** Do not implement deprecated APIs. Target the modern (3.x+) behavior.

---

## Appendix B: FrankenPandas-Only Features (No pandas Equivalent)

| Feature | Crate | Description |
|---------|-------|-------------|
| Alignment-Aware Columnar Execution (AACE) | fp-index | Core architectural differentiator |
| Leapfrog triejoin | fp-index | Multi-way alignment in O(n*k) |
| Incremental View Maintenance (IVM) | fp-expr | Delta propagation for materialized views |
| Approximate sketches (HLL, KLL, CMS) | fp-groupby | Streaming approximate analytics |
| CrackIndex (database cracking) | fp-columnar | Adaptive indexing on filter predicates |
| ValidityMask (packed bitvec) | fp-columnar | Memory-efficient null tracking |
| RuntimePolicy (strict/hardened) | fp-runtime | Security-conscious runtime decisions |
| ConformalGuard | fp-runtime | Statistical coverage monitoring |
| EvidenceLedger / GalaxyBrainCard | fp-runtime | Decision audit trail |
| Conformance harness + parity gates | fp-conformance | Automated behavioral parity testing |
| RaptorQ erasure coding for artifacts | fp-conformance | Fault-tolerant artifact storage |
| Dense bucket aggregation path | fp-groupby | Performance optimization for dense int keys |
| Borrowed-key HashMap alignment | fp-index | Memory optimization for union alignment |
| OnceCell memoization for has_duplicates | fp-index | 87% faster repeated duplicate checks |

---

*End of DOC-PASS-02: Symbol/API Census and Surface Classification*
*Total pandas public API symbols enumerated: ~545*
*FrankenPandas coverage: ~65 symbols (~12% overall, ~47% of CORE tier)*
