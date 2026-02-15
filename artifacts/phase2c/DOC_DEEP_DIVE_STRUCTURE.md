# DOC-PASS-14: Full-Agent Deep Dive Pass A -- Structure Specialist

**Bead:** bd-2gi.23.15
**Date:** 2026-02-14
**Status:** Complete
**Synthesized From:** All 10 crate Cargo.toml files, all 10 lib.rs files, DOC_PANDAS_STRUCTURE_EXPANSION, DOC_EXECUTION_PATHS, DOC_API_CENSUS, DOC_MODULE_CARTOGRAPHY
**Target:** Structural fidelity report auditing FrankenPandas crate decomposition against pandas subsystem structure

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Crate-to-Subsystem Mapping Matrix](#2-crate-to-subsystem-mapping-matrix)
3. [Subsystem Boundary Analysis](#3-subsystem-boundary-analysis)
4. [Type Fidelity Audit](#4-type-fidelity-audit)
5. [API Surface Census](#5-api-surface-census)
6. [Dependency Graph Audit](#6-dependency-graph-audit)
7. [Structural Gaps Analysis](#7-structural-gaps-analysis)
8. [Pattern Fidelity](#8-pattern-fidelity)
9. [Recommendations](#9-recommendations)
10. [Appendix: Full Public API Listing](#10-appendix-full-public-api-listing)

---

## 1. Executive Summary

FrankenPandas decomposes a ~310,000 LOC Python/Cython library into 10 Rust crates totaling ~17,337 LOC across ~188 public API items. The architectural strategy is deliberate: rather than replicating pandas' deep class hierarchy and mutable-state BlockManager, FP uses Rust's ownership model to enforce invariants at compile time, eliminates Copy-on-Write tracking entirely, and replaces the BlockManager with per-column `BTreeMap<String, Column>` storage.

### Scale Comparison

| Metric | pandas | FrankenPandas | Ratio |
|--------|--------|---------------|-------|
| Total non-test LOC | ~310,643 | ~17,337 | 5.6% |
| Public symbols | ~545 | ~188 | 34.5% |
| CORE-tier API coverage | ~85 symbols | ~40 covered | 47% |
| Type hierarchy depth | 6 layers (EA, Block, Manager, NDFrame, DF/Series, API) | 3 layers (types, columnar+index, frame) | Flatter |
| Crate/package count | 51 packages | 10 crates | 5:1 |
| Test files | 1,120 | 311 tests across 9 crates | -- |
| Error types | 1 hierarchy (`errors/`) + ad-hoc exceptions | 7 domain-specific `thiserror` enums | Stronger typing |

### Key Structural Findings

1. **No BlockManager.** FP replaces pandas' `BlockManager` (6,874 LOC across 6 modules) with a `BTreeMap<String, Column>` per DataFrame (`fp-frame/src/lib.rs:617`). This eliminates consolidation, block splitting, `_blknos`/`_blklocs` indirection, and the `PerformanceWarning` at 100+ blocks.

2. **No Copy-on-Write.** Pandas 3.0 tracks block references via `BlockValuesRefs` for lazy copy semantics. FP relies entirely on Rust ownership and borrowing -- data is owned, borrowed (`&`), or explicitly cloned. No weak-reference tracking, no `has_reference()` checks.

3. **Scalar-level vs array-level typing.** Pandas stores homogeneous numpy arrays or ExtensionArrays per block. FP stores `Vec<Scalar>` per column where each `Scalar` is a 5-variant tagged enum (`fp-types/src/lib.rs:26`). A `ColumnData` enum (`fp-columnar/src/lib.rs:183`) provides vectorized fast paths for homogeneous data.

4. **Explicit null discrimination.** Pandas conflates `np.nan`, `pd.NA`, and `pd.NaT` based on dtype context. FP's `NullKind` enum (`fp-types/src/lib.rs:18`) distinguishes all three explicitly, enabling lossless round-trip serialization.

5. **AACE as first-class architecture.** Alignment-Aware Columnar Execution is not bolted on as in pandas (`NDFrame.align` in `core/generic.py`), but baked into the `fp-index` crate as the primary computation primitive: `align()`, `align_union()`, `align_inner()`, `align_left()` (`fp-index/src/lib.rs:461-547`).

6. **Dependency DAG is strictly layered.** FP's crate dependency graph forms a clean DAG with no circular dependencies. Pandas has documented `_libs -> core` and `core -> io` circular import violations (DOC_PANDAS_STRUCTURE_EXPANSION Section 6.2).

7. **No God Objects.** Pandas' `NDFrame` (12,788 LOC in `core/generic.py`) and `DataFrame` (18,679 LOC in `core/frame.py`) are God Objects containing alignment, indexing, metadata, aggregation, and I/O dispatch. FP decomposes these across `fp-index` (alignment), `fp-columnar` (storage), `fp-frame` (container), `fp-groupby` (aggregation), `fp-join` (merging), and `fp-io` (serialization).

---

## 2. Crate-to-Subsystem Mapping Matrix

### 2.1 Primary Mapping

| FP Crate | LOC | pandas Subsystem(s) | pandas LOC | Coverage Depth |
|----------|-----|---------------------|-----------|----------------|
| `fp-types` | 628 | `core/dtypes/` (9,102) + `core/nanops.py` (1,777) + `errors/` (1,154) | 12,033 | PARTIAL -- 5 of ~30 dtypes, 8 nanops, unified error model |
| `fp-columnar` | 1,926 | `core/arrays/` (30,774) + `core/internals/` (6,874) + `core/ops/` (1,347) | 38,995 | PARTIAL -- single Column type vs 12+ array classes |
| `fp-index` | 1,612 | `core/indexes/` (21,816) + `_libs/index.pyx` (1,325) + `_libs/join.pyx:align` | 23,141+ | PARTIAL -- flat Index only, no MultiIndex/DatetimeIndex |
| `fp-frame` | 2,375 | `core/frame.py` (18,679) + `core/series.py` (9,860) + `core/generic.py` (12,788) | 41,327 | PARTIAL -- core ops, no loc/iloc/apply/sort |
| `fp-groupby` | 2,614 | `core/groupby/` (13,045) + `_libs/groupby.pyx` (2,325) | 15,370 | PARTIAL -- 10 agg funcs, no transform/filter/apply |
| `fp-join` | 1,103 | `core/reshape/merge.py` (3,135) + `_libs/join.pyx` (880) | 4,015 | PARTIAL -- 4 join types, no multi-key/asof/cross |
| `fp-io` | 909 | `io/` (47,386) | 47,386 | MINIMAL -- CSV + JSON only of 20+ formats |
| `fp-expr` | 520 | `core/computation/` (3,877) | 3,877 | MINIMAL -- Add expr + IVM, no eval/query |
| `fp-runtime` | 919 | (no pandas equivalent) | 0 | NOVEL -- Bayesian decision, AACE policy |
| `fp-conformance` | 4,731 | `_testing/` (2,813) | 2,813 | NOVEL -- conformance harness, parity gates, RaptorQ |

### 2.2 Unmapped pandas Subsystems

These pandas subsystems have no FP crate mapping and represent structural gaps:

| pandas Subsystem | LOC | Priority | Notes |
|-----------------|-----|----------|-------|
| `core/window/` | 6,771 | MEDIUM | Rolling, expanding, EWM -- no FP equivalent |
| `core/strings/` | 5,433 | MEDIUM | `.str` accessor -- no FP equivalent |
| `core/reshape/` (excl. merge) | 5,420 | MEDIUM | pivot, melt, stack, concat orchestration |
| `core/indexing.py` | 3,384 | HIGH | loc/iloc dispatch -- no FP equivalent |
| `core/resample.py` | 3,188 | LOW | Time-based resampling |
| `core/tools/` | 1,939 | MEDIUM | `to_datetime`, `to_numeric` converters |
| `core/interchange/` | 1,991 | LOW | DataFrame interchange protocol |
| `core/_numba/` | 1,770 | LOW | JIT compilation infrastructure |
| `core/algorithms.py` | 1,712 | MEDIUM | `unique`, `factorize`, `isin`, `rank` |
| `core/sorting.py` | 736 | HIGH | `nargsort`, `lexsort_indexer` |
| `_libs/tslibs/` | 26,418 | LOW (V2) | Time series Cython library |
| `plotting/` | 9,470 | OUT OF SCOPE | Visualization |
| `_config/` | 1,263 | LOW | Configuration system |

### 2.3 Cross-Subsystem Overlap

Several FP crates map to portions of multiple pandas subsystems:

```
fp-types  -------> core/dtypes/  (type algebra)
          -------> core/nanops.py  (NA-safe aggregation)
          -------> errors/  (error hierarchy)

fp-columnar ----> core/arrays/  (storage)
            ----> core/internals/  (block management)
            ----> core/ops/  (binary operations)
            ----> core/array_algos/  (take, filter)

fp-index  -------> core/indexes/  (label management)
          -------> _libs/index.pyx  (hash engines)
          -------> core/generic.py:align  (alignment primitive)

fp-frame  -------> core/frame.py  (DataFrame)
          -------> core/series.py  (Series)
          -------> core/generic.py  (NDFrame shared behavior)
          -------> core/reshape/concat.py  (concatenation)
```

---

## 3. Subsystem Boundary Analysis

### 3.1 Crate Dependency DAG

```
Layer 0 (leaf):       fp-types
                      /       \
Layer 1:         fp-columnar   fp-index
                    \    |       /
Layer 2:         fp-frame   fp-runtime
                /   |    \    /   \
Layer 3:  fp-groupby  fp-join  fp-io  fp-expr
                \      |       /       /
Layer 4:        fp-conformance (integration)
```

**Source:** All `Cargo.toml` files in `crates/fp-*/Cargo.toml`.

### 3.2 Boundary Contracts

| Boundary | Direction | Contract | Enforced By |
|----------|-----------|----------|-------------|
| `fp-types -> fp-columnar` | upward | `DType`, `Scalar`, `NullKind`, `TypeError`, nanops functions | Cargo.toml dependency |
| `fp-types -> fp-index` | upward | `DType` (not currently used; `IndexLabel` is independent) | Cargo.toml dependency |
| `fp-columnar -> fp-frame` | upward | `Column`, `ColumnData`, `ValidityMask`, `ArithmeticOp`, `ComparisonOp`, `ColumnError` | Cargo.toml dependency |
| `fp-index -> fp-frame` | upward | `Index`, `IndexLabel`, `AlignmentPlan`, `AlignMode`, `IndexError`, alignment functions | Cargo.toml dependency |
| `fp-runtime -> fp-frame` | upward | `RuntimePolicy`, `EvidenceLedger`, `RuntimeMode`, `DecisionAction` | Cargo.toml dependency |
| `fp-frame -> fp-groupby` | upward | `Series`, `DataFrame`, `FrameError` | Cargo.toml dependency |
| `fp-frame -> fp-join` | upward | `Series`, `FrameError` | Cargo.toml dependency |
| `fp-frame -> fp-io` | upward | `DataFrame`, `FrameError` | Cargo.toml dependency |
| `fp-frame -> fp-expr` | upward | `Series`, `FrameError` (via `fp_frame`) | Cargo.toml dependency |

### 3.3 Boundary Violation Audit

**Result: No violations found.** The FP crate graph is a strict DAG. No crate imports from a crate that depends on it.

**Contrast with pandas:**
- pandas has 5+ documented `_libs -> core` upward violations (DOC_PANDAS_STRUCTURE_EXPANSION, Section 6.2)
- pandas has `core -> io` violations for formatting (16+ import sites)
- These violations require deferred (inside-function) imports to break import cycles

**FP avoidance mechanism:** Rust's crate system enforces the DAG at compile time. A circular dependency is a compile error, not a runtime issue. The `fp-types` crate serves as the interface crate that all others can depend on without creating cycles.

### 3.4 Boundary Tightness Assessment

| Boundary | Items Crossing | Assessment |
|----------|---------------|------------|
| `fp-types` exports | 21 pub items (3 enums, 1 error, 17 functions) | TIGHT -- pure value types and pure functions |
| `fp-columnar` exports | 7 pub items (3 structs, 2 enums, 1 error, 1 struct) | TIGHT -- minimal surface |
| `fp-index` exports | 15 pub items (2 structs, 3 enums, 1 error, 9 functions) | MODERATE -- alignment functions could be methods |
| `fp-frame` exports | 5 pub items (2 structs, 1 error, 2 free functions) | TIGHT -- minimal surface for large crate |
| `fp-groupby` exports | 24 pub items (3 structs, 2 enums, 1 const, 14 functions, 4 sketch types) | MODERATE -- many free functions |
| `fp-join` exports | 9 pub items (3 structs, 2 enums, 1 const, 3 functions) | TIGHT |
| `fp-io` exports | 12 pub items (1 error, 2 enums, 1 struct, 8 functions) | MODERATE |
| `fp-expr` exports | 7 pub items (3 structs, 1 enum, 1 error, 1 function) | TIGHT |
| `fp-runtime` exports | 20 pub items (many structs, enums, 1 function) | MODERATE |
| `fp-conformance` exports | 68 pub items | WIDE -- integration/test infrastructure |

### 3.5 pandas Boundary Comparison

| pandas Boundary | pandas Violation Status | FP Equivalent | FP Status |
|-----------------|----------------------|---------------|-----------|
| `_libs -> core` (upward) | VIOLATED (5+ sites) | `fp-types`/`fp-columnar` -> `fp-frame` | CLEAN (no upward deps) |
| `core -> io` (upward) | VIOLATED (22+ sites) | `fp-frame` -> `fp-io` | CLEAN (io depends on frame, not reverse) |
| `core/dtypes` <- all of `core` | Heavy coupling (73 imports from arrays alone) | `fp-types` <- all crates | CLEAN (leaf dependency) |
| `io.formats.printing` misplaced | VIOLATED (16 core imports) | `Display` trait impls in `fp-types`, `fp-frame` | CLEAN (traits are in the right crates) |

---

## 4. Type Fidelity Audit

### 4.1 DType Mapping

| pandas DType | pandas LOC | FP Type | FP Location | Fidelity |
|-------------|-----------|---------|-------------|----------|
| `int8/16/32/64` | `core/arrays/integer.py:296` | `DType::Int64` | `fp-types/src/lib.rs:11` | PARTIAL -- single width |
| `uint8/16/32/64` | `core/arrays/integer.py:296` | (none) | -- | MISSING |
| `float32/64` | `core/arrays/floating.py:192` | `DType::Float64` | `fp-types/src/lib.rs:12` | PARTIAL -- single width |
| `bool_` / `BooleanDtype` | `core/arrays/boolean.py:438` | `DType::Bool` | `fp-types/src/lib.rs:10` | FULL |
| `object` / `StringDtype` | `core/arrays/string_.py:1232` | `DType::Utf8` | `fp-types/src/lib.rs:13` | FULL |
| `datetime64[ns]` | `core/arrays/datetimes.py:3123` | (none) | -- | MISSING |
| `timedelta64[ns]` | `core/arrays/timedeltas.py:1310` | (none) | -- | MISSING |
| `period` | `core/arrays/period.py:1493` | (none) | -- | MISSING |
| `CategoricalDtype` | `core/arrays/categorical.py:3194` | (none) | -- | MISSING |
| `IntervalDtype` | `core/arrays/interval.py:1889` | (none) | -- | MISSING |
| `SparseDtype` | `core/arrays/sparse/array.py:1993` | (none) | -- | MISSING |
| `ArrowDtype` | `core/arrays/arrow/array.py:3417` | (none) | -- | MISSING |
| `NumpyEADtype` | `core/arrays/numpy_.py:652` | (none) | -- | N/A |
| `DType::Null` | (no pandas equivalent as explicit dtype) | `DType::Null` | `fp-types/src/lib.rs:9` | NOVEL |

**Type width gap:** FP uses a single `Int64` and `Float64` width. Pandas supports 4 integer widths (8/16/32/64), 4 unsigned widths, and 2 float widths (32/64). This means FP cannot represent memory-efficient narrow columns or unsigned integer semantics.

### 4.2 Scalar Mapping

| pandas Scalar | FP Scalar | Location | Fidelity |
|--------------|-----------|----------|----------|
| Python `int` / `np.int64` | `Scalar::Int64(i64)` | `fp-types/src/lib.rs:29` | FULL |
| Python `float` / `np.float64` | `Scalar::Float64(f64)` | `fp-types/src/lib.rs:30` | FULL |
| Python `bool` / `np.bool_` | `Scalar::Bool(bool)` | `fp-types/src/lib.rs:28` | FULL |
| Python `str` | `Scalar::Utf8(String)` | `fp-types/src/lib.rs:31` | FULL |
| `np.nan` | `Scalar::Null(NullKind::NaN)` | `fp-types/src/lib.rs:27` | FULL |
| `pd.NA` | `Scalar::Null(NullKind::Null)` | `fp-types/src/lib.rs:27` | FULL |
| `pd.NaT` | `Scalar::Null(NullKind::NaT)` | `fp-types/src/lib.rs:27` | PARTIAL -- kind exists but no datetime support |
| `pd.Timestamp` | (none) | -- | MISSING |
| `pd.Timedelta` | (none) | -- | MISSING |
| `pd.Period` | (none) | -- | MISSING |
| `pd.Interval` | (none) | -- | MISSING |
| `pd.Categorical` | (none) | -- | MISSING |

### 4.3 Container Type Mapping

| pandas Container | FP Container | pandas Location | FP Location | Fidelity |
|-----------------|-------------|-----------------|-------------|----------|
| `DataFrame` | `DataFrame` | `core/frame.py:455` | `fp-frame/src/lib.rs:617` | PARTIAL |
| `Series` | `Series` | `core/series.py:1` | `fp-frame/src/lib.rs:29` | PARTIAL |
| `Index` | `Index` | `core/indexes/base.py:1` | `fp-index/src/lib.rs:108` | PARTIAL |
| `RangeIndex` | `Index::from_range()` | `core/indexes/range.py:1` | `fp-index/src/lib.rs` (method) | PARTIAL |
| `MultiIndex` | (none) | `core/indexes/multi.py:1` | -- | MISSING |
| `BlockManager` | `BTreeMap<String, Column>` | `core/internals/managers.py:1` | `fp-frame/src/lib.rs:619` | REPLACED |
| `Block` / `NumpyBlock` | `Column` | `core/internals/blocks.py:1` | `fp-columnar/src/lib.rs:442` | REPLACED |
| `ExtensionArray` | (no hierarchy) | `core/arrays/base.py:1` | -- | REPLACED (single Column type) |
| `BaseMaskedArray` | `ValidityMask` | `core/arrays/masked.py:1` | `fp-columnar/src/lib.rs:10` | ANALOGOUS |
| `NDFrame` | (none -- decomposed) | `core/generic.py:1` | -- | DECOMPOSED |

### 4.4 Error Type Mapping

| pandas Error | FP Error | FP Location |
|-------------|----------|-------------|
| `ValueError` | `FrameError::LengthMismatch` | `fp-frame/src/lib.rs:16` |
| `KeyError` | `IndexError` variants | `fp-index/src/lib.rs:439` |
| `TypeError` | `TypeError` | `fp-types/src/lib.rs:113` |
| `ParserError` | `IoError::Csv` | `fp-io/src/lib.rs:20` |
| `MergeError` | `JoinError` | `fp-join/src/lib.rs:27` |
| `PerformanceWarning` | (not needed -- no BlockManager) | -- |
| `SettingWithCopyWarning` | (not needed -- Rust ownership) | -- |
| `ChainedAssignmentError` | (not needed -- Rust ownership) | -- |

---

## 5. API Surface Census

### 5.1 Quantitative Summary

| FP Crate | pub structs | pub enums | pub functions | pub consts | Total pub items |
|----------|------------|-----------|---------------|------------|----------------|
| `fp-types` | 0 | 3 (`DType`, `NullKind`, `Scalar`) | 17 | 0 | 21 (includes 1 error enum) |
| `fp-columnar` | 2 (`ValidityMask`, `Column`) | 4 (`ColumnData`, `ArithmeticOp`, `ComparisonOp`, `ColumnError`) | 0 (methods only) | 0 | 7 (includes `CrackIndex`) |
| `fp-index` | 2 (`Index`, `AlignmentPlan`) | 4 (`IndexLabel`, `AlignMode`, `DuplicateKeep`, `IndexError`) | 9 | 0 | 15 (includes `MultiAlignmentPlan`) |
| `fp-frame` | 2 (`Series`, `DataFrame`) | 1 (`FrameError`) | 2 (`concat_series`, `concat_dataframes`) | 0 | 5 |
| `fp-groupby` | 5 (`GroupByOptions`, `GroupByExecutionOptions`, `HyperLogLog`, `KllSketch`, `CountMinSketch`) | 2 (`GroupByError`, `AggFunc`) | 15 | 1 | 24 (includes `SketchResult`) |
| `fp-join` | 3 (`JoinedSeries`, `JoinExecutionOptions`, `MergedDataFrame`) | 2 (`JoinType`, `JoinError`) | 3 | 1 | 9 |
| `fp-io` | 1 (`CsvReadOptions`) | 2 (`IoError`, `JsonOrient`) | 9 | 0 | 12 |
| `fp-expr` | 3 (`SeriesRef`, `EvalContext`, `MaterializedView`) | 2 (`Expr`, `ExprError`) | 1 (`evaluate`) | 0 | 7 (includes `Delta`) |
| `fp-runtime` | 11 | 4 | 2 | 0 | 20 (includes asupersync-gated items) |
| `fp-conformance` | ~30 | ~8 | ~30 | 0 | ~68 |
| **TOTAL** | | | | | **~188** |

### 5.2 Method Density Comparison

| Container | pandas methods | FP methods | Coverage |
|-----------|---------------|------------|----------|
| `DataFrame` | ~250 | 23 | 9.2% |
| `Series` | ~200 | 33 | 16.5% |
| `Index` | ~120 | 29 | 24.2% |
| `GroupBy` | ~80 | 15 (as free functions) | 18.8% |
| `Window` | ~50 | 0 | 0% |

### 5.3 Coverage by Functional Domain

| Domain | pandas symbol count | FP covered (FULL+PARTIAL) | Percentage |
|--------|-------------------|--------------------------|------------|
| Construction | 8 | 5 | 62.5% |
| Arithmetic/comparison | 22 | 10 | 45.5% |
| Aggregation | 30 | 8 | 26.7% |
| Missing data | 8 | 4 | 50.0% |
| Index operations | 50 | 22 | 44.0% |
| GroupBy aggregation | 43 | 10 | 23.3% |
| Join/merge | 5 | 4 | 80.0% |
| IO read | 21 | 2 | 9.5% |
| IO write | 20 | 2 | 10.0% |
| Sorting | 7 | 0 | 0% |
| Reshaping | 14 | 2 | 14.3% |
| loc/iloc indexing | 4 | 0 | 0% |
| Window functions | 50 | 0 | 0% |
| String accessor | 60 | 0 | 0% |
| DateTime accessor | 35 | 0 | 0% |

---

## 6. Dependency Graph Audit

### 6.1 Inter-Crate Dependency Matrix

Source: All 10 `crates/fp-*/Cargo.toml` files.

| Crate | Depends on FP crates | External deps |
|-------|---------------------|---------------|
| `fp-types` | (none -- leaf) | serde, thiserror |
| `fp-columnar` | fp-types | serde, thiserror |
| `fp-index` | fp-types | serde, thiserror |
| `fp-frame` | fp-columnar, fp-index, fp-runtime, fp-types | serde, thiserror |
| `fp-groupby` | fp-columnar, fp-frame, fp-index, fp-runtime, fp-types | bumpalo, thiserror |
| `fp-join` | fp-columnar, fp-frame, fp-index, fp-types | bumpalo, thiserror |
| `fp-io` | fp-columnar, fp-frame, fp-index, fp-types | csv, serde_json, thiserror |
| `fp-expr` | fp-columnar, fp-frame, fp-index, fp-runtime, fp-types | serde, thiserror |
| `fp-runtime` | (none for core; optional asupersync) | serde, serde_json, thiserror |
| `fp-conformance` | fp-columnar, fp-frame, fp-groupby, fp-index, fp-io, fp-join, fp-runtime, fp-types | raptorq, serde, serde_json, serde_yaml, sha2, thiserror |

### 6.2 Dependency Graph Properties

**Graph type:** Directed Acyclic Graph (DAG)
**Maximum depth:** 4 (fp-types -> fp-columnar -> fp-frame -> fp-groupby -> fp-conformance)
**Leaf crates:** fp-types, fp-runtime
**Root crate:** fp-conformance (depends on 8 of 9 other crates; does not depend on fp-expr)
**Fan-in leader:** fp-types (depended on by 8 crates)
**Fan-out leader:** fp-conformance (depends on 8 crates)

### 6.3 Topological Sort Order

Build order (any valid topological ordering):

```
1. fp-types        (leaf)
2. fp-runtime      (leaf -- no FP deps for core build)
3. fp-columnar     (depends on fp-types)
4. fp-index        (depends on fp-types)
5. fp-frame        (depends on fp-columnar, fp-index, fp-runtime, fp-types)
6. fp-groupby      (depends on fp-columnar, fp-frame, fp-index, fp-runtime, fp-types)
7. fp-join         (depends on fp-columnar, fp-frame, fp-index, fp-types)
8. fp-io           (depends on fp-columnar, fp-frame, fp-index, fp-types)
9. fp-expr         (depends on fp-columnar, fp-frame, fp-index, fp-runtime, fp-types)
10. fp-conformance (depends on 8 crates)
```

Steps 6-9 are independent of each other and can build in parallel.

### 6.4 External Dependency Audit

| External Crate | Used By | Purpose | pandas Equivalent |
|---------------|---------|---------|-------------------|
| `serde` | 7 crates | Serialization/deserialization | Python pickle, `__getstate__` |
| `thiserror` | 9 crates | Error derive macro | Python exceptions |
| `bumpalo` | fp-groupby, fp-join | Arena allocation | (no equivalent -- Python GC) |
| `csv` | fp-io | CSV parsing | `_libs/parsers.pyx` (C extension, 2,182 LOC) |
| `serde_json` | fp-io, fp-runtime | JSON handling | vendored ujson (4,988 LOC C) |
| `raptorq` | fp-conformance | Erasure coding | (no equivalent) |
| `sha2` | fp-conformance | Cryptographic hashing | (no equivalent) |
| `serde_yaml` | fp-conformance | YAML config parsing | (no equivalent) |

**Total external dependencies:** 8 direct crates (excluding workspace-level). This is notably minimal compared to pandas' 15+ required dependencies (numpy, python-dateutil, pytz/zoneinfo) and 30+ optional dependencies (pyarrow, openpyxl, sqlalchemy, numba, etc.).

### 6.5 Comparison with pandas Dependency Topology

| Property | pandas | FrankenPandas |
|----------|--------|---------------|
| Circular dependencies | YES (`_libs -> core`, `core -> io`) | NO |
| Maximum import depth | ~8 (api -> frame -> generic -> internals -> blocks -> arrays -> dtypes -> _libs) | 4 |
| God Objects | YES (`NDFrame` 12,788 LOC, `DataFrame` 18,679 LOC) | NO (largest crate pub surface: 5 items for fp-frame) |
| Layering violations | 5+ documented | 0 |
| Interface crate | None (types scattered) | fp-types (clean leaf) |
| Feature-gated deps | 30+ optional via `import_optional_dependency()` | 1 (asupersync via Cargo feature) |

---

## 7. Structural Gaps Analysis

### 7.1 Critical Structural Gaps (blocks core workflows)

#### Gap S1: No loc/iloc Indexing Layer

**pandas:** `core/indexing.py` (3,384 LOC) provides `_LocIndexer` and `_iLocIndexer` with complex dispatch trees for label-based and positional access (DOC_EXECUTION_PATHS, Section 7).

**FP status:** No equivalent. Users must manually call `index.position(&label)` then index into column values. No slice semantics, no boolean mask alignment through indexing API.

**Impact:** Blocks the most common pandas selection patterns (`df.loc[rows, cols]`, `df.iloc[i:j]`).

**Recommendation:** Create `LocAccessor` and `IlocAccessor` types in `fp-frame` with methods matching the pandas dispatch tree. Estimated: 400-600 LOC.

#### Gap S2: No Sorting Infrastructure

**pandas:** `core/sorting.py` (736 LOC) provides `nargsort`, `get_group_index`, `lexsort_indexer`. `DataFrame.sort_values()` and `Series.sort_values()` are among the most-used methods.

**FP status:** `Index::sort_values()` exists (`fp-index/src/lib.rs`) and `Index::argsort()` exists, but there is no `Series.sort_values()` or `DataFrame.sort_values()`.

**Impact:** Blocks nearly all analysis workflows that require ordered output.

**Recommendation:** Add `sort_values()` and `sort_index()` to `Series` and `DataFrame` in `fp-frame`. Estimated: 150-250 LOC.

#### Gap S3: No apply/map/transform

**pandas:** `core/apply.py` (2,147 LOC) provides `FrameApply`, `SeriesApply`, `GroupByApply`. These are CORE-tier APIs used for custom transformations.

**FP status:** No equivalent. FP's Rust ownership model makes closures less ergonomic than Python lambdas, but `Fn(&Scalar) -> Scalar` signatures are feasible.

**Impact:** Blocks custom transformation workflows.

**Recommendation:** Add `Series::map(f: impl Fn(&Scalar) -> Scalar)` and `DataFrame::apply(f, axis)` in `fp-frame`. Estimated: 200-400 LOC.

### 7.2 Moderate Structural Gaps

#### Gap S4: No Window Function Subsystem

**pandas:** `core/window/` (6,771 LOC) across 7 modules provides rolling, expanding, and EWM operations.

**FP status:** No equivalent crate. Would require a new `fp-window` crate.

**Impact:** Blocks time-series analysis workflows.

**Recommendation:** Create `fp-window` crate with `Rolling`, `Expanding` structs and core aggregations (sum, mean, std, min, max). Estimated: 800-1200 LOC.

#### Gap S5: No String Accessor

**pandas:** `core/strings/accessor.py` (~5,433 LOC) provides 60+ vectorized string operations via `.str` accessor.

**FP status:** No equivalent. FP has `DType::Utf8` and `Scalar::Utf8(String)` but no vectorized string operations.

**Impact:** Blocks text processing workflows.

**Recommendation:** Add a `StringOps` module to `fp-columnar` or a new `fp-string` crate. Start with CORE-tier: `contains`, `replace`, `lower`, `upper`, `strip`, `split`, `len`. Estimated: 600-900 LOC.

#### Gap S6: No Reshaping Operations

**pandas:** `core/reshape/` (8,555 LOC minus merge) provides pivot, melt, stack, unstack, get_dummies, cut/qcut.

**FP status:** `concat_series` and `concat_dataframes` exist in `fp-frame`. No pivot, melt, stack, or unstack.

**Impact:** Blocks data restructuring workflows.

**Recommendation:** Add `pivot_table` and `melt` as free functions in `fp-frame` or a new `fp-reshape` crate. Estimated: 500-800 LOC.

#### Gap S7: No MultiIndex

**pandas:** `core/indexes/multi.py` (4,807 LOC) provides hierarchical indexing with levels, codes, partial key matching.

**FP status:** `Index` supports only flat labels via `IndexLabel::Int64 | IndexLabel::Utf8` (`fp-index/src/lib.rs:12`). No levels, no codes, no hierarchical operations.

**Impact:** Blocks hierarchical grouping and multi-level index workflows.

**Recommendation:** Defer to V2. MultiIndex is complex (4,807 LOC in pandas) and not required for V1 CORE-tier coverage.

### 7.3 Minor Structural Gaps

#### Gap S8: No set_index / reset_index

**pandas:** These methods convert between column data and index data. `set_index` is used in nearly every data loading workflow.

**FP status:** DataFrame is constructed with an explicit index. No post-construction index manipulation.

**Recommendation:** Add `set_index(column_name)` and `reset_index()` to `DataFrame` in `fp-frame`. Estimated: 100-200 LOC.

#### Gap S9: No value_counts / describe

**pandas:** `value_counts()` and `describe()` are CORE-tier EDA functions.

**FP status:** No equivalent. Could be implemented using existing groupby_count infrastructure.

**Recommendation:** Add `value_counts()` to `Series` using `groupby_count`. Add `describe()` using existing `count/mean/std/min/max/median`. Estimated: 150-300 LOC.

#### Gap S10: Single-Width Numeric Types

**FP status:** Only `Int64` and `Float64`. No `Int8/16/32`, `UInt*`, or `Float32`.

**Impact:** Higher memory usage for columns that could use narrower types. Cannot represent unsigned semantics.

**Recommendation:** Extend `DType` and `Scalar` enums. This is a pervasive change affecting all crates. Defer to V2.

---

## 8. Pattern Fidelity

### 8.1 Alignment-Aware Computation

**pandas pattern:** Binary operations between Series with different indexes trigger automatic alignment via `NDFrame.align()` (union join by default). The result contains the union of both indexes, with NaN filled for positions where one Series lacks a label.

**FP implementation:** `fp-index/src/lib.rs:519` (`align_union`) builds an `AlignmentPlan` with `union_labels`, `left_positions`, and `right_positions`. Series arithmetic methods (`fp-frame/src/lib.rs:107`) call `align_union`, then reindex both columns using the position maps, then perform element-wise operation.

**Fidelity:** HIGH. The core semantic -- alignment before computation -- is faithfully reproduced. FP adds a `RuntimePolicy` gate (`fp-frame/src/lib.rs:156`, `add_with_policy`) that can reject operations in strict mode (e.g., duplicate-label alignment). Pandas has no equivalent safety gate.

**Divergence:** FP always does outer (union) alignment. Pandas defaults to outer but `align()` accepts `join='inner'|'left'|'right'|'outer'` parameter. FP has `align()` with `AlignMode` (`fp-index/src/lib.rs:446`) supporting all four modes, but `Series` arithmetic hardcodes union alignment.

### 8.2 GroupBy Split-Apply-Combine

**pandas pattern:** `df.groupby('key')` creates a lazy `GroupBy` object. Calling `.sum()` triggers `_cython_agg_general` which dispatches to Cython kernels. The `GroupBy` object can be reused for multiple aggregations.

**FP implementation:** `fp-groupby/src/lib.rs:58` (`groupby_sum`) is a free function taking `keys: &Series, values: &Series`. No lazy `GroupBy` object. Each aggregation is a separate function call.

**Fidelity:** MODERATE. The split-apply-combine result is correct, but the API shape differs:
- No method chaining (`df.groupby('key').sum().mean()`)
- No lazy grouper reuse (each call re-computes group assignments)
- No `transform`, `filter`, or `apply` support
- `groupby_agg` with `AggFunc` enum (`fp-groupby/src/lib.rs:447`) provides 10 aggregation variants

**Enhancement over pandas:** FP adds dense Int64 bucket fast path, borrowed-key `GroupKeyRef` for zero-clone hashing, bumpalo arena allocation, and approximate sketches (HLL, KLL, CMS) (`fp-groupby/src/lib.rs:775-1108`).

### 8.3 Join/Merge

**pandas pattern:** `pd.merge()` creates a `_MergeOperation` object that resolves keys, selects hash-join vs sort-merge algorithm, computes indexers, and reindexes both sides.

**FP implementation:** `fp-join/src/lib.rs:58` (`join_series`) and `fp-join/src/lib.rs:360` (`merge_dataframes`) implement all four join types via HashMap-based join. No sort-merge path.

**Fidelity:** MODERATE. All four join types (Inner, Left, Right, Outer) are correct. Missing:
- Multi-key joins
- Asof joins
- Cross joins
- Sort-merge optimization for sorted inputs
- Indicator column support
- Suffix customization (partial -- `JoinExecutionOptions`)

### 8.4 NA Propagation

**pandas pattern:** NA propagation is dtype-dependent. Float uses IEEE 754 NaN propagation. Nullable integer uses `pd.NA` with Kleene logic. Object dtype treats `None` and `np.nan` as missing.

**FP implementation:** All NA propagation goes through `Scalar::is_missing()` (`fp-types/src/lib.rs:47`), which returns true for `Scalar::Null(_)` and `Scalar::Float64(NaN)`. Arithmetic operations check `is_missing()` uniformly regardless of column dtype (`fp-columnar/src/lib.rs`).

**Fidelity:** HIGH for the common case. FP's unified NA handling avoids the float-promotion-on-NaN-insertion behavior that surprises pandas users. However, FP does not implement Kleene logic for nullable boolean operations (`True & NA == NA`, `False & NA == False`).

### 8.5 Index Immutability and Caching

**pandas pattern:** Index objects are immutable. Properties like `is_unique`, `is_monotonic_increasing`, and `_engine` are computed lazily and cached via `@cache_readonly` (DOC_PANDAS_STRUCTURE_EXPANSION, Section 5.3).

**FP implementation:** `Index` struct (`fp-index/src/lib.rs:108`) uses `OnceCell` for `duplicate_cache` and `sort_order_cache`. The `has_duplicates()` method computes and memoizes the result. `position()` adaptively selects binary search vs linear scan based on cached sort order detection.

**Fidelity:** HIGH. Both systems use lazy memoization for expensive properties. FP's `OnceCell` is the Rust equivalent of pandas' `@cache_readonly`. The AG-05 optimization yielded 87.2% faster repeated `has_duplicates` checks.

### 8.6 Column Storage

**pandas pattern:** `BlockManager` groups same-dtype columns into 2D numpy blocks. `Block.values` is a 2D array where rows = within-block column index, columns = row index. `_blknos`/`_blklocs` arrays map logical column positions to block/within-block positions.

**FP implementation:** `DataFrame` uses `BTreeMap<String, Column>` (`fp-frame/src/lib.rs:619`). Each `Column` independently stores `dtype`, `values: Vec<Scalar>`, and `validity: ValidityMask` (`fp-columnar/src/lib.rs:442`).

**Fidelity:** REPLACED. FP intentionally does not replicate the BlockManager. Trade-offs:

| Aspect | pandas BlockManager | FP BTreeMap<String, Column> |
|--------|-------------------|---------------------------|
| Same-dtype column ops | Fast (contiguous 2D block) | Per-column iteration (no cross-column cache locality) |
| Mixed-dtype insertion | Slow (block splitting + rebuild) | O(log n) BTreeMap insert |
| Column access by name | O(1) via `_blknos`/`_blklocs` | O(log n) via BTreeMap |
| Memory overhead | Low (shared block metadata) | Higher (per-column metadata) |
| Consolidation cost | Required periodically | None |
| Fragmentation warnings | Yes (100+ blocks) | None |
| Column ordering | Insertion order | Alphabetical (BTreeMap sort) |

### 8.7 Copy Semantics

**pandas pattern:** Pandas 3.0 enforces Copy-on-Write. `df[col]` returns a view; mutations trigger lazy copy. `BlockValuesRefs` tracks weak references between blocks sharing data.

**FP implementation:** Rust ownership model. `Series::clone()` deep-copies all data. No views, no shared references beyond borrows. All operations return new values.

**Fidelity:** REPLACED. This is a deliberate architectural divergence. Rust's borrow checker provides compile-time guarantees that pandas' CoW provides at runtime. Trade-off: FP may use more memory for operations that would be views in pandas (column selection, slicing), but eliminates the entire class of view-vs-copy bugs.

---

## 9. Recommendations

### 9.1 Priority 1: Structural Completions (unblocks V1 workflows)

| # | Action | Target Crate | Est. LOC | Blocked Workflows |
|---|--------|-------------|---------|-------------------|
| R1 | Add `sort_values()`, `sort_index()` to Series and DataFrame | fp-frame | 150-250 | Nearly all analysis |
| R2 | Add `LocAccessor` / `IlocAccessor` with dispatch | fp-frame | 400-600 | Selection, mutation |
| R3 | Add `Series::map(f)`, `DataFrame::apply(f, axis)` | fp-frame | 200-400 | Custom transforms |
| R4 | Add `set_index()`, `reset_index()` to DataFrame | fp-frame | 100-200 | Index management |
| R5 | Add `value_counts()`, `describe()` to Series | fp-frame | 150-300 | EDA |
| R6 | Add `any()`, `all()` boolean reductions | fp-frame | 50-100 | Boolean logic |

### 9.2 Priority 2: New Subsystems (enables new domains)

| # | Action | Target | Est. LOC | New Domain |
|---|--------|--------|---------|------------|
| R7 | Create `fp-window` crate (rolling, expanding) | new crate | 800-1200 | Time series |
| R8 | Add string operations module | fp-columnar or new | 600-900 | Text processing |
| R9 | Add pivot_table, melt | fp-frame or new | 500-800 | Reshaping |
| R10 | Add multi-key join support | fp-join | 200-400 | Complex merges |

### 9.3 Priority 3: Type System Expansion (V2)

| # | Action | Impact |
|---|--------|--------|
| R11 | Add `Int8/16/32`, `UInt*`, `Float32` to `DType` and `Scalar` | Memory efficiency, unsigned semantics |
| R12 | Add datetime/timedelta types | Time-series workflows |
| R13 | Add `MultiIndex` to `fp-index` | Hierarchical indexing |
| R14 | Add `CategoricalDtype` | Memory-efficient categoricals |

### 9.4 Architectural Observations

1. **The fp-frame crate is the bottleneck.** At 2,375 LOC with only 5 pub items, it has the highest LOC-to-surface-area ratio. Most Priority 1 recommendations (R1-R6) target `fp-frame`. Consider splitting into `fp-series` and `fp-dataframe` if it grows beyond ~4,000 LOC.

2. **Free functions vs methods.** `fp-groupby` uses free functions (`groupby_sum`, `groupby_mean`, etc.) rather than methods on a `GroupBy` struct. This prevents method chaining and lazy grouper reuse. Consider wrapping in a `GroupBy` struct in a future pass.

3. **Column ordering divergence.** FP's `BTreeMap<String, Column>` sorts columns alphabetically. Pandas preserves insertion order. This can cause behavioral divergence in column iteration, display, and `from_dict` output. Consider using `IndexMap` or `Vec<(String, Column)>` to preserve insertion order.

4. **fp-runtime has no dependents at Layer 3+.** Only `fp-frame` and `fp-expr` depend on `fp-runtime` for policy checking. The `fp-groupby` crate also depends on it, but `fp-join` and `fp-io` do not. If policy enforcement is to be universal, all Layer 3 crates should depend on `fp-runtime`.

5. **fp-conformance missing fp-expr dependency.** The conformance crate (`fp-conformance/Cargo.toml:7-14`) depends on 8 FP crates but not `fp-expr`. Expression evaluation cannot be conformance-tested without this dependency.

---

## 10. Appendix: Full Public API Listing

### 10.1 fp-types (628 LOC, 21 pub items)

**File:** `crates/fp-types/src/lib.rs`

```
Enums:
  DType{Null, Bool, Int64, Float64, Utf8}                    line 8
  NullKind{Null, NaN, NaT}                                   line 18
  Scalar{Null(NullKind), Bool(bool), Int64(i64),
         Float64(f64), Utf8(String)}                          line 26
  TypeError{IncompatibleTypes, HomogeneityViolation,
            CastError, InferenceError}                        line 113

Functions:
  common_dtype(left, right) -> Result<DType>                  line 130
  infer_dtype(values) -> Result<DType>                        line 146
  cast_scalar_owned(value, target) -> Result<Scalar>          line 156
  cast_scalar(value, target) -> Result<Scalar>                line 209
  isna(values) -> Vec<bool>                                   line 215
  notna(values) -> Vec<bool>                                  line 219
  count_na(values) -> usize                                   line 223
  fill_na(values, fill) -> Vec<Scalar>                        line 227
  dropna(values) -> Vec<Scalar>                               line 240
  nansum(values) -> Scalar                                    line 254
  nanmean(values) -> Scalar                                   line 262
  nancount(values) -> Scalar                                  line 271
  nanmin(values) -> Scalar                                    line 276
  nanmax(values) -> Scalar                                    line 284
  nanmedian(values) -> Scalar                                 line 292
  nanvar(values, ddof) -> Scalar                              line 306
  nanstd(values, ddof) -> Scalar                              line 316

Key methods on Scalar:
  dtype() -> DType                                            line 36
  is_missing() -> bool                                        line 47
```

### 10.2 fp-columnar (1,926 LOC, 7 pub items)

**File:** `crates/fp-columnar/src/lib.rs`

```
Structs:
  ValidityMask { words: Vec<u64>, len: usize }                line 10
  Column { dtype: DType, values: Vec<Scalar>,
           validity: ValidityMask }                           line 442
  CrackIndex { column_ref: String, pieces: Vec<CrackPiece> } line 868

Enums:
  ColumnData{Float64(Vec<f64>), Int64(Vec<i64>),
             Bool(Vec<bool>), Utf8(Vec<String>)}              line 183
  ArithmeticOp{Add, Sub, Mul, Div}                            line 450
  ComparisonOp{Gt, Lt, Eq, Ne, Ge, Le}                        line 463
  ColumnError{TypeMismatch, LengthMismatch, OutOfBounds,
              InvalidMask, Homogeneity, CrackError}           line 473

Key Column methods:
  new(dtype, values) -> Result<Column>
  from_values(values: Vec<Scalar>) -> Result<Column>
  binary_numeric(left, right, op) -> Result<Column>
  binary_comparison(left, right, op) -> Result<Column>
  compare_scalar(scalar, op) -> Result<Column>
  filter_by_mask(mask) -> Column
  fillna(fill) -> Column
  dropna() -> Column
  reindex_by_positions(positions) -> Column

Key ValidityMask methods:
  from_values(values) -> Self                                 line 17
  all_valid(len) -> Self                                      line 30
  all_invalid(len) -> Self                                    line 42
  is_valid(idx) -> bool
  count_valid() -> usize
  count_invalid() -> usize
```

### 10.3 fp-index (1,612 LOC, 15 pub items)

**File:** `crates/fp-index/src/lib.rs`

```
Enums:
  IndexLabel{Int64(i64), Utf8(String)}                        line 12
  AlignMode{Inner, Left, Right, Outer}                        line 446
  DuplicateKeep{First, Last, None}                            line 101
  IndexError{DuplicateLabels, EmptyIndex, LabelNotFound,
             AlignmentViolation}                              line 439

Structs:
  Index { labels: Vec<IndexLabel>,
          duplicate_cache: OnceCell<bool>,
          sort_order_cache: OnceCell<SortOrder> }             line 108
  AlignmentPlan { union_labels, left_positions,
                  right_positions }                           line 432
  MultiAlignmentPlan { union_labels, position_maps }          line 562

Free functions:
  align(left, right, mode) -> AlignmentPlan                   line 461
  align_inner(left, right) -> AlignmentPlan                   line 478
  align_left(left, right) -> AlignmentPlan                    line 501
  align_union(left, right) -> AlignmentPlan                   line 519
  validate_alignment_plan(plan) -> Result<()>                 line 548
  leapfrog_union(indexes) -> Index                            line 572
  leapfrog_intersection(indexes) -> Index                     line 626
  multi_way_align(indexes) -> MultiAlignmentPlan              line 703

Key Index methods:
  new(labels) -> Self
  from_range(start, end) -> Self
  len() -> usize
  position(label) -> Option<usize>        (adaptive: binary search or linear)
  position_map_first() -> HashMap
  contains(label) -> bool
  get_indexer(target) -> Vec<Option<usize>>
  has_duplicates() -> bool                (OnceCell memoized)
  is_sorted() -> bool
  isin(values) -> Vec<bool>
  unique() -> Self
  duplicated(keep) -> Vec<bool>
  drop_duplicates(keep) -> Self
  intersection(other) -> Self
  union_with(other) -> Self
  difference(other) -> Self
  symmetric_difference(other) -> Self
  argsort() -> Vec<usize>
  sort_values() -> Self
  take(indices) -> Self
  slice(start, end) -> Self
```

### 10.4 fp-frame (2,375 LOC, 5 pub items)

**File:** `crates/fp-frame/src/lib.rs`

```
Enums:
  FrameError{LengthMismatch, DuplicateIndexUnsupported,
             CompatibilityRejected, Column, Index}            line 15

Structs:
  Series { name: String, index: Index, column: Column }       line 29
  DataFrame { index: Index,
              columns: BTreeMap<String, Column> }             line 617

Free functions:
  concat_series(series_list) -> Result<Series>                line 549
  concat_dataframes(frames) -> Result<DataFrame>              line 578

Key Series methods:
  new(name, index, column) -> Result<Self>                    line 36
  from_values(name, values) -> Result<Self>                   line 51
  from_pairs(name, pairs) -> Result<Self>                     line 65
  broadcast(name, index, scalar) -> Result<Self>              line 76
  name() -> &str                                              line 87
  index() -> &Index                                           line 92
  column() -> &Column                                         line 97
  values() -> &[Scalar]                                       line 102
  add/sub/mul/div(other) -> Result<Self>                      lines 165-207
  add_with_policy/sub_with_policy/...                         lines 156-207
  align(other, mode) -> Result<(Self, Self)>                  line 228
  combine_first(other) -> Result<Self>                        line 245
  reindex(new_labels) -> Result<Self>                         line 266
  gt/lt/eq_series/ne_series/ge/le(other) -> Result<Self>      lines 312-337
  compare_scalar(scalar, op) -> Result<Self>                  line 344
  filter(mask) -> Result<Self>                                line 356
  fillna(fill_value) -> Result<Self>                          line 382
  dropna() -> Result<Self>                                    line 390
  count() -> usize                                            line 408
  sum/mean/min/max/std/var/median() -> Result<Scalar>         lines 417-522

Key DataFrame methods:
  new(index, columns) -> Result<Self>                         line 623
  from_series(series_list) -> Result<Self>                    line 639
  from_dict(column_order, data) -> Result<Self>               line 674
  from_dict_with_index(column_order, data, index) -> Result   line 714
  select_columns(names) -> Result<Self>                       line 738
  len() -> usize                                              line 757
  is_empty() -> bool                                          line 763
  num_columns() -> usize                                      line 769
  index() -> &Index                                           line 774
  columns() -> &BTreeMap<String, Column>                      line 779
  column_names() -> Vec<&String>                              line 787
  column(name) -> Option<&Column>                             line 792
  filter_rows(mask) -> Result<Self>                           line 801
  head(n) -> Result<Self>                                     line 839
  tail(n) -> Result<Self>                                     line 853
  with_column(name, column) -> Result<Self>                   line 868
  drop_column(name) -> Result<Self>                           line 883
  rename_columns(mapping) -> Result<Self>                     line 897
```

### 10.5 fp-groupby (2,614 LOC, 24 pub items)

**File:** `crates/fp-groupby/src/lib.rs`

```
Structs:
  GroupByOptions { dropna: bool }                             line 14
  GroupByExecutionOptions { use_arena, arena_budget_bytes }   line 37
  HyperLogLog { registers, precision }                       line 775
  KllSketch { compactors, k, size }                          line 863
  CountMinSketch { table, width, depth }                     line 999
  SketchResult { estimate, relative_error }                  line 1056

Enums:
  GroupByError{Frame, Index, Column}                          line 25
  AggFunc{Sum, Mean, Count, Min, Max,
          First, Last, Std, Var, Median}                     line 447

Constants:
  DEFAULT_ARENA_BUDGET_BYTES: usize = 256MB                  line 34

Free functions:
  groupby_sum(keys, values, options, policy, ledger)          line 58
  groupby_sum_with_options(keys, values, opts, exec, p, l)   line 75
  groupby_agg(keys, values, func, options, policy, ledger)   line 469
  groupby_mean(keys, values, options, policy, ledger)        line 636
  groupby_count(keys, values, options, policy, ledger)       line 647
  groupby_min(keys, values, options, policy, ledger)         line 658
  groupby_max(keys, values, options, policy, ledger)         line 669
  groupby_first(keys, values, options, policy, ledger)       line 680
  groupby_last(keys, values, options, policy, ledger)        line 691
  groupby_std(keys, values, options, policy, ledger)         line 702
  groupby_var(keys, values, options, policy, ledger)         line 713
  groupby_median(keys, values, options, policy, ledger)      line 724
  approx_nunique(values) -> SketchResult                     line 1069
  approx_quantile(values, q) -> Option<SketchResult>         line 1090
  approx_value_counts(values) -> Vec<(Scalar, u64)>          line 1108
```

### 10.6 fp-join (1,103 LOC, 9 pub items)

**File:** `crates/fp-join/src/lib.rs`

```
Enums:
  JoinType{Inner, Left, Right, Outer}                        line 12
  JoinError{Frame, Column}                                   line 27

Structs:
  JoinedSeries { index, left_values, right_values }          line 20
  JoinExecutionOptions { use_arena, arena_budget_bytes }     line 37
  MergedDataFrame { index, columns }                         line 336

Constants:
  DEFAULT_ARENA_BUDGET_BYTES: usize = 256MB                  line 34

Free functions:
  join_series(left, right, join_type) -> Result<JoinedSeries> line 58
  join_series_with_options(left, right, type, opts)           line 66
  merge_dataframes(left, right, on, how) -> Result<Merged>   line 360
```

### 10.7 fp-io (909 LOC, 12 pub items)

**File:** `crates/fp-io/src/lib.rs`

```
Enums:
  IoError{MissingHeaders, JsonFormat, Csv, Json, Io,
          Utf8, Column, Frame}                               line 14
  JsonOrient{Records, Columns, Split}                        line 34

Structs:
  CsvReadOptions { delimiter, has_headers, na_values,
                   index_col }                               line 41

Free functions:
  read_csv_str(input) -> Result<DataFrame>                   line 59
  write_csv_string(frame) -> Result<String>                  line 98
  read_csv_with_options(input, options) -> Result<DataFrame>  line 175
  read_csv(path) -> Result<DataFrame>                        line 244
  write_csv(frame, path) -> Result<()>                       line 249
  read_json_str(input, orient) -> Result<DataFrame>          line 291
  write_json_string(frame, orient) -> Result<String>         line 412
  read_json(path, orient) -> Result<DataFrame>               line 477
  write_json(frame, path, orient) -> Result<()>              line 482
```

### 10.8 fp-expr (520 LOC, 7 pub items)

**File:** `crates/fp-expr/src/lib.rs`

```
Structs:
  SeriesRef(String)                                          line 12
  EvalContext { series: BTreeMap<String, Series> }           line 23
  Delta { series_name, new_labels, new_values }              line 80
  MaterializedView { expr, result, base_snapshot }           line 88

Enums:
  Expr{Series{name}, Add{left, right}, Literal{value}}      line 16
  ExprError{UnknownSeries, UnanchoredLiteral, Frame}         line 46

Free functions:
  evaluate(expr, context, policy, ledger) -> Result<Series>  line 55

Key MaterializedView methods:
  new(expr, context, policy, ledger) -> Result<Self>
  apply_delta(delta, policy, ledger) -> Result<()>
  result() -> &Series
```

### 10.9 fp-runtime (919 LOC, 20 pub items)

**File:** `crates/fp-runtime/src/lib.rs`

```
Enums:
  RuntimeMode{Strict, Hardened}                              line 10
  DecisionAction{Allow, Reject, Repair}                      line 17
  IssueKind{UnknownFeature, MalformedInput,
            JoinCardinality, PolicyOverride}                  line 25
  RuntimeError{PolicyViolation, OverflowError, ...}          line 259

Structs:
  CompatibilityIssue { kind, subject, detail }               line 33
  EvidenceTerm { name, log_likelihood_* }                    line 40
  LossMatrix { allow_if_compatible, allow_if_incompatible,
               reject_if_compatible, reject_if_incompatible } line 47
  DecisionMetrics { prior_log_odds, posterior_log_odds,
                    expected_loss_* }                         line 70
  DecisionRecord { issue, evidence, metrics, action, ts }    line 79
  GalaxyBrainCard { title, summary, evidence_lines,
                    metrics_summary, decision }              line 90
  EvidenceLedger { records: Vec<DecisionRecord> }            line 124
  RuntimePolicy { mode, duplicate_label_limit,
                  join_cardinality_limit, prior_log_odds,
                  default_loss_matrix }                      line 147
  RaptorQEnvelope { metadata, packets }                      line 325
  RaptorQMetadata { transfer_length, symbol_size,
                    content_sha256 }                         line 335
  ScrubStatus { ok, bad_indexes }                            line 343
  DecodeProof { sha256, decoded_len, status }                line 349
  ConformalPredictionSet { predictions, alpha, coverage }    line 393
  ConformalGuard { calibration_scores, alpha }               line 408

Functions:
  decision_to_card(record) -> GalaxyBrainCard                line 108
  outcome_to_action(outcome) -> DecisionAction               line 555 (asupersync-gated)
```

### 10.10 fp-conformance (4,731 LOC, ~68 pub items)

**File:** `crates/fp-conformance/src/lib.rs`

Key items (abbreviated):

```
Structs:
  HarnessConfig { repo_root, oracle_root, fixture_root, ... } line 26
  HarnessReport { smoke_ok, error_messages, ... }
  PacketFixture, PacketParityReport
  DifferentialResult, DriftRecord
  CiGate, CiPipelineResult
  E2eConfig, E2eReport
  ForensicLog, FailureForensicsReport
  RaptorQEnvelope handling utilities

Functions:
  run_smoke(config) -> HarnessReport
  run_parity_gate(fixture) -> PacketParityReport
  differential_test(...) -> DifferentialResult
  run_ci_gate(gate) -> CiPipelineResult
  run_e2e(config) -> E2eReport
  run_forensics(log) -> FailureForensicsReport
  (30+ additional pub functions for test orchestration)
```

---

## Appendix A: LOC Verification

Source: `wc -l crates/fp-*/src/lib.rs` (all source in single lib.rs per crate).

| Crate | LOC | % of Total |
|-------|-----|-----------|
| fp-conformance | 4,731 | 27.3% |
| fp-groupby | 2,614 | 15.1% |
| fp-frame | 2,375 | 13.7% |
| fp-columnar | 1,926 | 11.1% |
| fp-index | 1,612 | 9.3% |
| fp-join | 1,103 | 6.4% |
| fp-runtime | 919 | 5.3% |
| fp-io | 909 | 5.2% |
| fp-types | 628 | 3.6% |
| fp-expr | 520 | 3.0% |
| **Total** | **17,337** | **100%** |

## Appendix B: Invariant Mapping

| pandas Invariant | Source | FP Equivalent | FP Source |
|-----------------|--------|---------------|-----------|
| `len(index) == nrows` for all blocks | `managers.py:_verify_integrity` | `index.len() == column.len()` for all columns | `fp-frame/src/lib.rs:37` |
| DType homogeneity within block | `blocks.py:__init__` | DType homogeneity within Column | `fp-columnar/src/lib.rs:442` (Column::new validates) |
| Index uniqueness for reindex | `indexes/base.py:_validate_can_reindex` | `has_duplicates()` with OnceCell memoization | `fp-index/src/lib.rs:108` |
| NA propagation per dtype | `core/nanops.py` + `core/ops/mask_ops.py` | Unified via `Scalar::is_missing()` | `fp-types/src/lib.rs:47` |
| Sort stability (mergesort/Timsort) | All sort operations | Rust `sort_by` (Timsort, stable) | Standard library |
| CoW prevents unintended mutation | `BlockValuesRefs` + `has_reference()` | Rust ownership (compile-time guarantee) | Language-level |
| Block `_can_hold_na` flag | `blocks.py:_can_hold_na` | All dtypes hold NA via `Scalar::Null(_)` | `fp-types/src/lib.rs:27` |
| Exclusive column ownership | `_verify_integrity` checks coverage | BTreeMap guarantees unique keys | `fp-frame/src/lib.rs:619` |

---

*Generated for FrankenPandas Phase-2C, bead bd-2gi.23.15 (DOC-PASS-14). Structural fidelity audit comparing 10 FP crates (~17,337 LOC, ~188 pub items) against pandas (~310,643 LOC, ~545 symbols). All file:line references verified against source.*
