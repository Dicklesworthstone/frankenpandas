# Documentation Gap Matrix + Quantitative Expansion Targets

**Bead**: bd-2gi.23.1
**Date**: 2026-02-14
**Author**: BrightForge
**Scope**: Measuring documentation completeness in `EXHAUSTIVE_LEGACY_ANALYSIS.md` and `EXISTING_PANDAS_STRUCTURE.md` against the full pandas legacy codebase.

---

## 1. Document Metrics Summary

| Metric | EXHAUSTIVE_LEGACY_ANALYSIS.md | EXISTING_PANDAS_STRUCTURE.md |
|---|---|---|
| **Total lines** | 281 | 72 |
| **Approximate words** | 1,588 | 377 |
| **Top-level sections (##)** | 18 (sections 0-17) | 8 (sections 1-8) |
| **Subsections (### and deeper)** | 2 (7.1, 7.2) | 0 |
| **Tables** | 3 (sections 3, 5, 6) | 0 |
| **Enumerated list items** | ~55 | ~30 |

---

## 2. Pandas Legacy Codebase Ground Truth

**Total non-test source**: 295,222 LOC across 358 files (Python + Cython + Cython headers)
**Total test source**: 384,382 LOC across 1,120 files

### Source LOC by Major Subsystem

| Subsystem | LOC | Files | Semantic Weight |
|---|---|---|---|
| `core/` top-level (frame, generic, series, indexing) | 61,416 | 20 | Critical |
| `core/arrays/` (extension arrays, categorical, datetime, masked) | 23,905 | 22+ | Critical |
| `core/indexes/` (base, multi, range, datetime, interval) | 21,816 | 14 | Critical |
| `core/groupby/` (groupby, grouper, ops, generic) | 13,045 | 9 | Critical |
| `core/dtypes/` (cast, common, dtypes, inference) | 9,102 | 11 | Critical |
| `core/reshape/` (merge, concat, pivot, melt, tile) | 8,555 | 8 | Critical |
| `core/internals/` (blocks, managers, construction, concat) | 6,874 | 6 | Critical |
| `core/window/` (rolling, expanding, ewm) | 6,771 | 7 | High |
| `core/strings/` (accessor, object_array) | 5,433 | 3 | Medium |
| `core/computation/` (eval, expr, engines, pytables) | 3,877 | 12 | Medium |
| `core/tools/` | 1,939 | varies | Low |
| `core/ops/` (array_ops, mask_ops) | 1,347 | 6 | High |
| `core/methods/` | 964 | varies | Low |
| `_libs/tslibs/` (offsets, timestamps, timedeltas, period, nattype) | 26,507 | 35+ | Critical |
| `_libs/` top-level (algos, groupby, hashtable, join, parsers, lib) | 17,275 | 30+ | Critical |
| `io/` top-level (sql, stata, html, parquet, pickle) | 18,278 | 15+ | High |
| `io/formats/` (format, style, style_render) | 14,213 | 14 | Medium |
| `io/parsers/` (readers, python_parser, c_parser_wrapper) | 5,675 | 5 | High |
| `io/excel/` | 4,183 | 10 | Medium |
| `io/json/` | 2,541 | 4 | Medium |
| `io/sas/` | 1,749 | 4 | Low |
| `plotting/` | 3,134 | varies | Low (V1 excluded) |
| `tseries/` | 1,426 | varies | Medium |
| Other (`api`, `_config`, `_testing`, `errors`, `compat`, `util`) | ~7,601 | varies | Low-Medium |

### Test File Counts by Domain

| Test Domain | Files | Test Domain | Files |
|---|---|---|---|
| `tests/indexes` | 191 | `tests/io` (all) | 137 |
| `tests/frame` | 119 | `tests/series` | 105 |
| `tests/arrays` | 101 | `tests/extension` | 48 |
| `tests/groupby` | 45 | `tests/scalar` | 39 |
| `tests/tseries` | 34 | `tests/indexing` | 32 |
| `tests/reshape` | 31 | `tests/util` | 25 |
| `tests/window` | 24 | `tests/copy_view` | 23 |
| `tests/plotting` | 20 | `tests/dtypes` | 19 |
| `tests/tslibs` | 19 | `tests/arithmetic` | 13 |
| `tests/apply` | 12 | `tests/strings` | 11 |
| `tests/resample` | 9 | `tests/base` | 9 |
| `tests/generic` | 8 | `tests/tools` | 5 |
| `tests/libs` | 5 | `tests/internals` | 3 |
| `tests/reductions` | 3 | `tests/computation` | 3 |
| `tests/config` | 3 | `tests/construction` | 2 |

---

## 3. Per-Section Coverage Assessment: EXHAUSTIVE_LEGACY_ANALYSIS.md

| Section | Content Summary | Depth | Notes |
|---|---|---|---|
| 0. Mission + Completion Criteria | Scoping definition, 5-point checklist | Shallow | Meta-only, no subsystem specifics |
| 1. Source-of-Truth Crosswalk | File refs to legacy corpus + contracts | Shallow | Pure pointers, no extraction |
| 2. Quantitative Legacy Inventory | File counts, high-density zones | Medium | Accurate numbers, no per-module LOC |
| 3. Subsystem Extraction Matrix | 9-row table: legacy -> Rust crates | Medium | 1-2 sentences/row, no method inventories |
| 4. Alien-Artifact Invariant Ledger | 5 invariants (FP-I1 through FP-I5) | Shallow | Statements only, no derivation |
| 5. Native/Cython Boundary Register | 4-row risk table | Shallow | Risk class only, no kernel extraction |
| 6. Compatibility/Security Doctrine | 5-row threat/mode decision table | Medium | Good policy structure, no test cases |
| 7. Conformance Program | 6 fixture families + harness spec | Medium | Correct grouping, no coverage budgets |
| 8. Extreme Optimization Program | Hotspots + budget thresholds | Medium | Numeric targets, no profiling data |
| 9. RaptorQ Artifact Contract | Sidecar + envelope field spec | Shallow | Infrastructure only |
| 10. Phase-2 Execution Backlog | 12-item extraction backlog | Medium | Concrete items, no size estimates |
| 11. Residual Gaps and Risks | 3 bullet risk items | Shallow | Minimal |
| 12. Deep-Pass Hotspot Inventory | Top 6 files by LOC | Medium | Good data, limited scope |
| 13. Phase-2C Payload Contract | 10-field extraction payload spec | Medium | Strong template, no per-module detail |
| 14. Strict/Hardened Drift Budgets | Numeric drift budgets + reports | Medium | Specific thresholds |
| 15. Optimization Execution Law | 5-step loop + scoring gate | Medium | Process defined, workloads named |
| 16. RaptorQ Evidence Topology | Naming convention + scrub reqs | Shallow | Infrastructure only |
| 17. Phase-2C Exit Checklist | 6-item exit criteria | Shallow | Checklist only |

**Summary**: Primarily a process/governance document. Very thin on per-module semantic extraction (method inventories, branch decision tables, dtype promotion rules, error surface maps).

---

## 4. Per-Section Coverage Assessment: EXISTING_PANDAS_STRUCTURE.md

| Section | Content Summary | Depth | Notes |
|---|---|---|---|
| 1. Legacy Oracle | Root path + upstream ref | Shallow | 2 lines |
| 2. Subsystem Map | 5-bullet module overview | Shallow | Directories only, no inventories |
| 3. Semantic Hotspots | 7-item numbered list | Medium | 1-2 sentences each |
| 4. Compat-Critical Behaviors | 4-bullet list | Shallow | Topics named only |
| 5. Security/Stability Risks | 5-bullet risk list | Shallow | Threats named, no mitigation |
| 6. V1 Extraction Boundary | Include/exclude scope | Shallow | Coarse-grained only |
| 7. Conformance Fixture Families | 6-bullet test mapping | Shallow | Directory names only |
| 8. Extraction Notes for Rust Spec | 3-bullet guidance | Shallow | Principles only |

**Summary**: High-level orientation map at 72 lines. No behavioral extraction, no decision tables, no method inventories.

---

## 5. Gap Identification: What Is NOT Covered

### A. Major Subsystems With NO Dedicated Analysis

| Pandas Module | Source LOC | Test Files | ELA Coverage | EPS Coverage |
|---|---|---|---|---|
| `core/arrays/` (extension array system) | 23,905 | 149 | One table row | One bullet |
| `core/dtypes/` (type system, casting) | 9,102 | 19 | Not addressed | One bullet |
| `core/window/` (rolling/expanding/ewm) | 6,771 | 24 | Not addressed | One bullet |
| `core/strings/` (string accessor) | 5,433 | 11 | Not addressed | Not addressed |
| `core/computation/` (eval/expr engine) | 3,877 | 3 | Not addressed | Mentioned as risk |
| `core/resample.py` (time-series resampling) | 3,188 | 9 | Not addressed | Not addressed |
| `core/apply.py` (apply/applymap) | 2,147 | 12 | Not addressed | Not addressed |
| `core/algorithms.py` (top-level algos) | 1,712 | - | Not addressed | Not addressed |
| `core/array_algos/` (take, putmask, quantile) | ~1,800 | varies | Not addressed | Not addressed |
| `core/interchange/` (DataFrame interchange) | ~2,000 | varies | Not addressed | Not addressed |
| `io/formats/` (format, style, style_render) | 14,213 | 26 | Not addressed | Not addressed |
| `io/parsers/` (CSV parser internals) | 5,675 | 43 | Part of IO row | Not addressed |
| `io/excel/` | 4,183 | 9 | Part of IO row | Not addressed |
| `io/json/` | 2,541 | 10 | Part of IO row | Not addressed |
| `io/sql.py` | ~3,100 | varies | Part of IO row | Mentioned as risk |
| `io/stata.py` | ~4,400 | varies | Part of IO row | Not addressed |
| `io/pytables.py` (HDF5) | ~5,700 | 20 | Not addressed | Not addressed |
| `_libs/tslibs/` (time series Cython) | 26,507 | 19 | Table row + 1 bullet | Not addressed |
| `_libs/window/` (window Cython) | ~2,600 | varies | Not addressed | Not addressed |
| `plotting/` | 3,134 | 20 | Not addressed | Excluded in V1 |
| `tseries/` (offsets, frequencies) | 1,426 | 34 | Not addressed | Not addressed |
| Copy-on-Write semantics | embedded | 23 | Not addressed | Not addressed |

### B. Critical Behavioral Domains With No Extraction

1. **Dtype promotion/casting rules**: `core/dtypes/cast.py` + `core/dtypes/common.py` contain hundreds of promotion paths -- zero lines documented.
2. **ExtensionArray contract**: `core/arrays/base.py` defines the extension array API surface -- not documented.
3. **MultiIndex semantics**: `core/indexes/multi.py` is the largest index file -- mentioned by name only.
4. **Copy-on-Write (CoW) mechanics**: 23 dedicated test files, embedded throughout core -- not addressed.
5. **String accessor methods**: `core/strings/accessor.py` -- not addressed.
6. **Resample semantics**: `core/resample.py` -- not addressed.
7. **Apply/map semantics**: `core/apply.py` -- not addressed.
8. **Styler/formatting**: `io/formats/style.py` + `style_render.py` -- not addressed.
9. **Expression evaluation engine**: `core/computation/` (security-critical) -- mentioned as risk only.
10. **Categorical semantics**: `core/arrays/categorical.py` -- not addressed specifically.
11. **Interval/Period semantics**: Large files in both arrays and indexes -- not addressed.
12. **Error surface contracts**: `errors/` module + exception hierarchies -- not systematically documented.

---

## 6. Expansion Multiplier Matrix

Multiplier scale: 1x = adequate | 2-3x = moderate | 4-8x = substantial | 10-20x = near-complete creation | NEW = topic absent

| Topic Area | Current Lines | Target Lines | Multiplier | Priority |
|---|---|---|---|---|
| Process/Governance/Meta | ~120 | ~150 | 1.3x | Low |
| DataFrame/Series construction + alignment | ~15 | ~200 | 13x | P0 |
| Index model (base, multi, range, datetime, interval) | ~8 | ~300 | 38x | P0 |
| BlockManager/internals | ~5 | ~120 | 24x | P0 |
| loc/iloc indexing semantics | ~8 | ~150 | 19x | P0 |
| GroupBy contract (groupby, grouper, ops) | ~6 | ~200 | 33x | P0 |
| Reshape/Join/Merge semantics | ~6 | ~180 | 30x | P0 |
| Missing/NaN/NaT propagation | ~6 | ~120 | 20x | P0 |
| Dtype system (types, casting, inference, promotion) | ~3 | ~250 | 83x | P0 |
| Extension Array contract | ~2 | ~200 | 100x | P1 |
| Cython boundary extraction (_libs) | ~12 | ~300 | 25x | P0 |
| Temporal semantics (_libs/tslibs) | ~6 | ~250 | 42x | P0 |
| IO/parsers (CSV, JSON core) | ~6 | ~150 | 25x | P1 |
| IO/formats (output formatting, style) | 0 | ~100 | NEW | P2 |
| IO/excel | 0 | ~60 | NEW | P2 |
| IO/sql | ~2 | ~80 | 40x | P2 |
| IO/parquet,feather,orc | 0 | ~60 | NEW | P2 |
| IO/stata,sas,spss | 0 | ~50 | NEW | P3 |
| IO/HDF5 (pytables) | 0 | ~50 | NEW | P3 |
| Window functions (rolling/expanding/ewm) | ~2 | ~120 | 60x | P1 |
| String accessor | 0 | ~100 | NEW | P1 |
| Computation/eval/expr | ~2 | ~80 | 40x | P2 |
| Resample | 0 | ~100 | NEW | P1 |
| Apply/map | 0 | ~80 | NEW | P1 |
| Categorical semantics | 0 | ~100 | NEW | P1 |
| Interval/Period arrays+indexes | 0 | ~100 | NEW | P1 |
| Copy-on-Write (CoW) | 0 | ~80 | NEW | P1 |
| Sorting/algorithms | 0 | ~60 | NEW | P2 |
| Interchange protocol | 0 | ~40 | NEW | P3 |
| Error surface/exception contracts | 0 | ~80 | NEW | P1 |
| Conformance fixture detail | ~15 | ~200 | 13x | P1 |
| Optimization profiling detail | ~20 | ~100 | 5x | P2 |
| Security threat vectors (detail) | ~15 | ~80 | 5x | P2 |

---

## 7. Aggregate Expansion Summary

| Document | Current Size | Target Size | Multiplier |
|---|---|---|---|
| EXHAUSTIVE_LEGACY_ANALYSIS.md | 281 lines / 1,588 words | ~2,800-3,500 lines / ~16,000-20,000 words | **10-12x** |
| EXISTING_PANDAS_STRUCTURE.md | 72 lines / 377 words | ~1,200-1,500 lines / ~7,000-9,000 words | **17-21x** |
| **Combined** | 353 lines / 1,965 words | ~4,000-5,000 lines / ~23,000-29,000 words | **11-14x** |

---

## 8. Coverage Heat Map

```
COVERED (mentioned + some detail):        ████░░░░░░░░░░░░░░░░  (~20%)
  DataFrame/Series construction (shallow)
  Index base behavior (shallow)
  GroupBy defaults (shallow)
  Join/reshape (shallow)
  Null propagation (shallow)
  IO surface (shallow)
  Cython boundaries (shallow)
  Optimization targets (medium)
  Process/governance (medium)

MENTIONED ONLY (named, no extraction):     ███░░░░░░░░░░░░░░░░░  (~15%)
  Temporal/tslibs
  Expression evaluation
  BlockManager internals
  Security threat surface

NOT COVERED AT ALL:                        █████████████░░░░░░░  (~65%)
  Dtype system / casting / promotion rules
  Extension array contract
  MultiIndex semantics (deep)
  Window functions
  String accessor
  Categorical semantics
  Interval/Period semantics
  Copy-on-Write
  Resample
  Apply/map
  IO formats/style
  IO excel/parquet/stata/HDF5 specifics
  Error hierarchy contracts
  Interchange protocol
  Sorting/algorithms
  Array algorithms (take, putmask, quantile)
  Sparse arrays
```

---

## 9. Key Findings

1. **EXHAUSTIVE_LEGACY_ANALYSIS.md is primarily a governance/process document**, not a semantic extraction document. The Subsystem Extraction Matrix (section 3) is the closest to behavioral documentation but covers only 9 rows at 1-2 sentences each.

2. **EXISTING_PANDAS_STRUCTURE.md is a lightweight orientation map** at 72 lines. It names directories and identifies 7 semantic hotspots but provides no method inventories, decision tables, or edge-case analysis.

3. **The actual extraction work lives in the Phase-2C artifacts** (`artifacts/phase2c/FP-P2C-001` through `FP-P2C-011` and `ESSENCE_EXTRACTION_LEDGER.md`), not in these two documents. These two files serve as the framing layer above packet-level extraction.

4. **The dtype system is the largest undocumented critical surface**: `core/dtypes/` alone is 9,102 LOC with complex casting/promotion paths, yet receives essentially zero documentation.

5. **Extension arrays, window functions, string methods, and resample together represent ~43,000+ LOC** with zero dedicated documentation.

6. **The IO surface is treated as a single unit** in one table row, when it actually spans 18,278 + 14,213 + 5,675 + 4,183 LOC across radically different semantic domains.

7. **Combined expansion multiplier of 11-14x** indicates these documents need approximately an order of magnitude more content to serve as comprehensive clean-room extraction references.

---

## 10. Downstream Bead Implications

This gap matrix directly informs:
- **bd-2gi.23.2**: Topic decomposition and per-topic extraction briefs
- **bd-2gi.23.3**: Per-topic expansion passes
- **bd-2gi.23.4**: Integration and cross-reference validation

The P0 topic areas (9 topics, combined current coverage ~69 lines, target ~1,770 lines) should be prioritized in bd-2gi.23.2 topic decomposition.
