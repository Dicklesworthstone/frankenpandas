# FrankenPandas

<div align="center">
  <img src="frankenpandas_illustration.webp" alt="FrankenPandas - Clean-room Rust reimplementation of pandas" width="600">

  **Clean-room Rust reimplementation of the full pandas API surface.**

  Drop-in pandas API in safe Rust — today. Python bindings (`import frankenpandas`) planned via PyO3; see the Roadmap. Zero `unsafe`. Profile-proven performance.

  ![Rust](https://img.shields.io/badge/Rust-2024_edition-orange)
  ![License](https://img.shields.io/badge/license-MIT-blue)
  ![Tests](https://img.shields.io/badge/tests-3200%2B-brightgreen)
  ![IO Formats](https://img.shields.io/badge/IO_formats-8-purple)
</div>

---

## TL;DR

**The Problem:** pandas is the lingua franca of data analysis, but it's single-threaded Python with unpredictable memory spikes, GIL contention in production pipelines, and dtype coercion surprises that silently corrupt results.

**The Solution:** FrankenPandas rebuilds the entire pandas API from first principles in Rust. Same semantics, same method names, same edge-case behavior, but with columnar storage, vectorized kernels, arena-backed execution, and compile-time safety guarantees.

**Why FrankenPandas?**

| Feature | pandas | Polars | FrankenPandas |
|---------|--------|--------|---------------|
| API compatibility with pandas | - | Partial (different API) | Full parity target |
| Memory safety | Runtime errors | Safe Rust | Safe Rust (`#![forbid(unsafe_code)]`) |
| Index alignment semantics | Yes | No (no index) | Yes (AACE) |
| GroupBy with named aggregation | Yes | Yes (different syntax) | Yes (`agg_named`) |
| `eval()`/`query()` string expressions | Yes | No | Yes |
| Column MultiIndex | Yes | No | Yes (foundation) |
| Row MultiIndex | Yes | No | Yes (DataFrame/groupby/indexing/reshape/IO integration) |
| Categorical dtype | Yes | Yes | Yes (metadata layer) |
| 8 IO formats (CSV/JSON/JSONL/Parquet/Excel/SQL/Feather/IPC) | Yes (SQL: any SQLAlchemy engine) | Partial | Yes (SQL: generic `SqlConnection`, SQLite default backend; PostgreSQL/MySQL planned) |
| Conformance testing against pandas oracle | - | - | Yes (430+ packet suites, 1250 fixtures; 557 lib packets passing, 5 known divergences tracked in DISCREPANCIES.md DISC-011 through DISC-014) |

## Quick Example

```rust
use frankenpandas::prelude::*;

// Read CSV (with pandas-style options)
let df = read_csv_str("name,age,city\nAlice,30,NYC\nBob,25,LA\nCarol,35,NYC")?;

// Filter with query expressions (just like df.query() in pandas)
let adults = df.query("age > 28")?;

// GroupBy with named aggregation
let summary = adults.groupby(&["city"])?.agg_named(&[
    ("avg_age", "age", "mean"),
    ("count", "age", "count"),
])?;

// Export to any format
let json = write_json_string(&summary, JsonOrient::Records)?;
let feather = write_feather_bytes(&summary)?;

println!("{}", summary);
// city    avg_age  count
// NYC       32.5      2
```

## Crown-Jewel Innovation

**Alignment-Aware Columnar Execution (AACE):** Every binary operation between DataFrames or Series goes through an explicit index-alignment planning phase before any data is touched. An `EvidenceLedger` records each materialization decision with Bayesian confidence scores, creating a fully auditable execution trace.

AACE is a core identity constraint, not a best-effort optimization. pandas' alignment semantics (outer join on index for arithmetic, left join for assignment) are preserved exactly, with formal correctness evidence.

## Design Philosophy

| Principle | What It Means |
|-----------|---------------|
| **Semantic parity over speed** | We never sacrifice pandas-observable behavior for performance. Every dtype promotion, NaN propagation, and output ordering contract is preserved. |
| **Prove, then optimize** | Each optimization round produces a baseline, an opportunity matrix, an isomorphism proof, and a recommendation contract. No optimization without correctness evidence. |
| **Fail closed** | Unknown features, incompatible dtypes, and ambiguous coercions produce errors, not silent corruption. Strict mode rejects; hardened mode logs and recovers. |
| **Zero unsafe** | Every crate uses `#![forbid(unsafe_code)]`. Memory safety comes from the type system, not audits. |
| **Test everything differentially** | Conformance packets run FrankenPandas operations and compare against the pandas oracle. Machine-readable parity reports catch regressions automatically. |

## Architecture

```
                  ┌────────────────────────┐
                  │     frankenpandas       │  ← Unified facade crate
                  │   (prelude re-exports)  │
                  └───────────┬────────────┘
                              │
       ┌──────────────────────┼──────────────────────┐
       ▼                      ▼                      ▼
  ┌──────────┐      ┌────────────────┐      ┌──────────────┐
  │ fp-expr   │      │   fp-frame     │      │    fp-io     │
  │ eval()    │      │ DataFrame      │      │  8 formats   │
  │ query()   │      │ Series         │      │  CSV/JSON/   │
  └─────┬─────┘      │ Categorical    │      │  JSONL/      │
        │            │ MultiIndex     │      │  Parquet/    │
        ▼            └───────┬────────┘      │  Excel/SQL/  │
  ┌──────────┐               │               │  Feather/IPC │
  │fp-runtime│        ┌──────┼──────┐        └──────────────┘
  │ Policy   │        ▼      ▼      ▼
  │ Ledger   │   fp-index fp-groupby fp-join
  └──────────┘   alignment  arena-   merge_asof
                  planning  backed   cross join
                            agg
       ┌────────────────────┘
       ▼
  ┌────────────┐      ┌───────────────┐
  │ fp-columnar │      │   fp-types    │
  │ Column      │      │ Scalar, DType │
  │ ValidMask   │      │ NaN/NaT/Null  │
  └────────────┘      └───────────────┘
```

The diagram above shows the 10 runtime/data-flow crates. Two auxiliary crates — `fp-conformance` (differential testing harness against the pandas oracle) and `fp-frankentui` (experimental terminal UI dashboard) — live alongside them and bring the workspace total to **12 crates**, 140,000+ lines of Rust (non-test), 1,600+ public functions.

## Workspace Structure

```
frankenpandas/
├── crates/
│   ├── frankenpandas/    # Unified facade crate with prelude
│   ├── fp-types/         # Scalar, DType, NullKind, coercion rules
│   ├── fp-columnar/      # Column, ValidityMask, vectorized kernels
│   ├── fp-index/         # Index, MultiIndex, alignment planning
│   ├── fp-frame/         # DataFrame, Series, Categorical, 960+ methods
│   ├── fp-expr/          # Expression parser, eval()/query()
│   ├── fp-groupby/       # GroupBy with 3 execution paths
│   ├── fp-join/          # Inner/Left/Right/Outer/Cross/Asof joins
│   ├── fp-io/            # 8 IO formats (CSV/JSON/JSONL/Parquet/Excel/Feather/IPC/SQL), 429 tests
│   ├── fp-conformance/   # Differential testing against pandas oracle
│   ├── fp-runtime/       # Strict/Hardened policy, EvidenceLedger
│   └── fp-frankentui/    # Terminal UI dashboard (experimental)
├── artifacts/perf/       # Optimization round evidence
└── artifacts/phase2c/    # Conformance packet artifacts
```

## IO Format Support

| Format | Read | Write | In-Memory | File | Options |
|--------|------|-------|-----------|------|---------|
| **CSV** | `read_csv_str` | `write_csv_string` | Yes | Yes | delimiter, headers, na_values, index_col, usecols, nrows, skiprows, dtype |
| **JSON** | `read_json_str` | `write_json_string` | Yes | Yes | 5 orients: Records, Columns, Index, Split, Values |
| **JSONL** | `read_jsonl_str` | `write_jsonl_string` | Yes | Yes | One object per line, blank-line tolerant, union-key detection |
| **Parquet** | `read_parquet_bytes` | `write_parquet_bytes` | Yes | Yes | Arrow RecordBatch integration, multi-batch reading |
| **Excel** | `read_excel_bytes` | `write_excel_bytes` | Yes | Yes | sheet_name, has_headers, index_col, skip_rows; .xlsx/.xls/.xlsb/.ods |
| **Feather** | `read_feather_bytes` | `write_feather_bytes` | Yes | Yes | Arrow IPC file format (random-access footer) |
| **Arrow IPC stream** | `read_ipc_stream_bytes` | `write_ipc_stream_bytes` | Yes | Yes | Streaming wire format (forward-only; pipes + zero-copy interchange) |
| **SQL** | `read_sql` / `read_sql_table` / `read_sql_chunks` | `write_sql` | N/A | `SqlConnection` trait; SQLite backend by default | `SqlReadOptions` (params, parse_dates, coerce_float, dtype, schema, columns, index_col), `SqlWriteOptions` (if_exists, index, index_label, schema, dtype, method, chunksize), `SqlInspector` for `SQLAlchemy.Inspector`-shaped introspection (tables / views / schemas / columns / indexes / FKs / UCs / reflect_table / reflect_all_tables) |

All formats accessible through `DataFrameIoExt` trait methods on DataFrame.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
frankenpandas = { path = "crates/frankenpandas" }
```

Or use individual crates for finer-grained control:

```toml
[dependencies]
fp-frame = { path = "crates/fp-frame" }
fp-io = { path = "crates/fp-io" }
fp-types = { path = "crates/fp-types" }
```

### Cargo features

The `frankenpandas` umbrella forwards inner-crate feature flags so callers can opt in/out without depending on inner crates directly:

| Feature | Default | Forwards to | What it gates |
|---------|---------|-------------|---------------|
| `sql-sqlite` | **on** | `fp-io/sql-sqlite` | rusqlite-backed `SqlConnection` impl. Drop this to build without the SQLite dep. |
| `sql-postgresql` | off | `fp-io/sql-postgresql` | Placeholder for the Phase 2 PostgreSQL adapter (no concrete bindings yet). |
| `sql-mysql` | off | `fp-io/sql-mysql` | Placeholder for the Phase 2 MySQL adapter. |
| `tracing` | off | `fp-frame/tracing` | Emits `tracing` spans on hot paths (groupby, rolling, resample, IO). |
| `asupersync` | off | `fp-runtime/asupersync` | Pulls in the optional `asupersync` runtime integration submodule. |

```toml
# Default — includes sql-sqlite
[dependencies]
frankenpandas = { path = "crates/frankenpandas" }

# No SQL deps — drop rusqlite + the sql-sqlite SqlConnection impl
frankenpandas = { path = "crates/frankenpandas", default-features = false }

# SQL + tracing spans
frankenpandas = { path = "crates/frankenpandas", features = ["tracing"] }
```

When `sql-sqlite` is disabled, `read_sql` / `write_sql` and the `SqlConnection` trait remain available — just implement the trait for your own connection type to route the same APIs through it.

### Build from Source

```bash
git clone https://github.com/Dicklesworthstone/frankenpandas.git
cd frankenpandas
cargo build --workspace --release
cargo test --workspace
```

Requires **Rust nightly** (2024 edition). See `rust-toolchain.toml`.

## Quick Start

```rust
use frankenpandas::prelude::*;

// 1. Create a DataFrame
let df = read_csv_str("ticker,price,volume\nAAPL,185.50,1000\nGOOG,140.25,500\nAAPL,186.00,1200")?;

// 2. Filter rows
let expensive = df.query("price > 150")?;

// 3. GroupBy and aggregate
let by_ticker = expensive.groupby(&["ticker"])?.sum()?;

// 4. Convert dates
let dates = Series::from_values("d", vec![0i64.into()], vec![Scalar::Utf8("2024-01-15".into())])?;
let parsed = to_datetime(&dates)?;

// 5. Export to multiple formats
let csv = write_csv_string(&by_ticker)?;
let json = write_json_string(&by_ticker, JsonOrient::Records)?;
let feather = write_feather_bytes(&by_ticker)?;

// 6. SQL round-trip
let conn = rusqlite::Connection::open_in_memory()?;
write_sql(&by_ticker, &conn, "results", SqlIfExists::Fail)?;
let back = read_sql_table(&conn, "results")?;
```

## How It Works: Deep Dive

### Data Model

FrankenPandas uses a columnar storage model identical to pandas' internal representation:

```
DataFrame
├── index: Index (Vec<IndexLabel>)        ← Row labels (Int64/Utf8/Datetime64/Timedelta64)
├── row_multiindex: Option<MultiIndex>    ← Optional hierarchical row index (set by set_index_multi)
├── columns: BTreeMap<String, Column>     ← Named columns
├── column_order: Vec<String>             ← Insertion-order preservation
└── column_multiindex: Option<MultiIndex> ← Optional hierarchical column header

Column
├── dtype: DType  ← {Null, Bool, Int64, Float64, Utf8, Categorical, Timedelta64, Sparse}
├── values: Vec<Scalar>  ← Typed values
└── validity: ValidityMask  ← Bitpacked null bitmap

Scalar = Null(NullKind) | Bool(bool) | Int64(i64) | Float64(f64) | Utf8(String) | Timedelta64(i64)
NullKind = Null | NaN | NaT  ← Three-way null semantics matching pandas
```

Every `Scalar` knows its own type, and every `Column` enforces type homogeneity through `DType`. The type promotion hierarchy follows pandas exactly: `Null < Bool < Int64 < Float64` (with Utf8 incompatible with numerics). `Categorical`, `Timedelta64`, and `Sparse` are extension dtypes layered on top of the core numeric/string buckets.

### ValidityMask: Bitpacked Null Tracking

Nulls are tracked using a packed bitmap, not per-element Option types:

```
ValidityMask {
    words: Vec<u64>,  // Each u64 holds 64 validity bits
    len: usize,       // Actual number of elements
}
```

- **Bit `1`** = valid, **Bit `0`** = null/missing
- Element at position `i` is in word `i / 64`, bit `i % 64`
- Last word is masked to clear unused high bits: `(1u64 << remainder) - 1`
- Boolean algebra: `and_mask`, `or_mask`, `not_mask` operate word-by-word on u64s, processing 64 null checks per CPU instruction
- `count_valid()` uses `word.count_ones()` (hardware `POPCNT`) across all words

This is the same approach used by Apache Arrow and gives O(n/64) performance for null-aware operations instead of O(n).

### Index Alignment: The AACE Pipeline

When you write `series_a + series_b` in pandas, it silently performs an outer join on the indexes before adding. FrankenPandas makes this explicit:

```
Step 1: Plan
   align_union(left.index, right.index) → AlignmentPlan {
       union_index: Index,          // Merged label set
       left_positions: Vec<Option<usize>>,   // Where each left value goes
       right_positions: Vec<Option<usize>>,  // Where each right value goes
   }

Step 2: Materialize
   Reindex both columns according to the plan.
   None positions → fill with NaN (missing propagation).

Step 3: Execute
   Vectorized element-wise operation on aligned arrays.

Step 4: Log
   EvidenceLedger records the decision: timestamp, mode, action,
   Bayesian posterior, expected losses.
```

Single-label `Index::position()` lookups use **adaptive search** (AG-13): each `Index` lazily detects its own sort order in a `std::sync::OnceLock<SortOrder>`, then dispatches to O(log n) binary search for sorted Int64/Utf8 labels and an O(n) linear scan for unsorted labels. The alignment planner itself (`align_union`) always builds borrowed-key HashMaps (AG-02) for the union construction regardless of sort order — the adaptive switch is on the lookup side, not the planner side.

For multi-way alignment (e.g., `DataFrame.from_series([s1, s2, s3])`), a leapfrog triejoin variant (AG-05) computes the N-way union in a single O(n log n) pass instead of iterative pairwise merges.

### GroupBy: Three Execution Paths

The GroupBy engine automatically selects the fastest execution path based on key cardinality and memory budget:

```
                       ┌────────────────────────┐
                       │  All keys Int64 AND    │
                       │  key range ≤ 65,536?   │
                       └───────────┬────────────┘
                          Yes      │       No
                    ┌──────────────┘       └──────────────┐
                    ▼                                      ▼
          ┌────────────────────┐         ┌──────────────────────────┐
          │ Dense Int64 Path   │         │  Estimated working set   │
          │ O(1) array index   │         │  fits arena budget       │
          │ Pre-alloc by key   │         │  (default 256 MB)?       │
          │ range: max-min     │         └────────────┬─────────────┘
          └────────────────────┘             Yes      │      No
                                       ┌──────────────┘      └──────────────┐
                                       ▼                                     ▼
                            ┌────────────────────┐           ┌──────────────────────┐
                            │ Arena-backed Path  │           │ HashMap Generic Path │
                            │ Bumpalo bump alloc │           │ (source_idx, sum)    │
                            │ single malloc      │           │ pairs, no key clone  │
                            │ bulk dealloc       │           │ (AG-08)              │
                            └────────────────────┘           └──────────────────────┘
```

**Path 1, Dense Int64:** When all group keys are integers spanning ≤ 65,536 values, pre-allocates a dense array indexed directly by `key - min_key`. O(1) per-element grouping with zero hash overhead. Used for common patterns like grouping by year, month, category ID.

**Path 2, Arena-backed (Bumpalo):** When estimated intermediate memory fits within the arena budget (default 256 MB), allocates all working memory from a Bumpalo bump allocator. Single `malloc` + pointer bumps; bulk deallocation when the arena drops. Zero fragmentation, cache-friendly.

**Path 3, Global allocator HashMap:** Fallback for arbitrary key types and unbounded cardinality. Stores `(source_index, accumulating_sum)` pairs and never clones the group key Scalar itself (AG-08 optimization). The original IndexLabel is reconstructed at output time from the source position.

All three paths produce identical output. This is verified by property-based tests that run the same inputs through arena and non-arena paths and assert bitwise equality.

### Vectorized Kernels

Arithmetic on typed arrays avoids per-element enum dispatch:

```rust
// Instead of matching each Scalar variant per element:
fn vectorized_binary_f64(
    left: &[f64], right: &[f64],
    left_validity: &ValidityMask, right_validity: &ValidityMask,
    op: fn(f64, f64) -> f64,
) -> (Vec<f64>, ValidityMask)
```

The compiler auto-vectorizes the inner loop to SIMD instructions. Combined validity is computed via `and_mask` on the bitmap words, again handling 64 elements per instruction.

When types don't align for the fast path (e.g., Int64 + Float64), a scalar fallback promotes each element through `cast_scalar()`, respecting the full pandas type hierarchy.

### Expression Engine

`df.eval("a + b * c > threshold")` and `df.query("price > 100 and volume < 500")` are powered by a recursive-descent parser in fp-expr:

```
Grammar (simplified):
  expr       → or_expr
  or_expr    → and_expr ( "or" and_expr )*
  and_expr   → comparison ( "and" comparison )*
  comparison → arithmetic ( (">" | "<" | "==" | ...) arithmetic )?
  arithmetic → term ( ("+" | "-") term )*
  term       → atom ( ("*" | "/") atom )*
  atom       → NUMBER | COLUMN_NAME | @LOCAL_VAR | "(" expr ")"
```

The parser produces an `Expr` AST that the evaluator walks, resolving column references against the DataFrame's `EvalContext`. Local variables (prefixed with `@`) are broadcast to Series of the appropriate length. The entire pipeline (parse, resolve, evaluate, filter) happens in a single call with no temporary DataFrames.

### Bayesian Runtime Policy

The runtime distinguishes between **strict mode** (fail on any ambiguity) and **hardened mode** (log and attempt repair). The decision between Allow, Reject, and Repair uses Bayesian expected-loss minimization:

```
For each compatibility issue:
  1. Start with prior P(compatible) based on issue type
     - Unknown feature: 0.25
     - Join admission: 0.60

  2. Update with evidence terms (log-likelihood ratios)
     - allowlist_miss:        compat=-3.5, incomp=-0.2
     - unknown_protocol_field: compat=-2.0, incomp=-0.1
     - overflow_risk:         compat=-0.3, incomp=-1.2

  3. Compute posterior via Bayes' theorem

  4. Calculate expected loss for each action:
     E[loss(Allow)]  = L(allow|compat) * P(compat) + L(allow|incomp) * P(incomp)
     E[loss(Reject)] = L(reject|compat) * P(compat) + L(reject|incomp) * P(incomp)
     E[loss(Repair)] = L(repair|compat) * P(compat) + L(repair|incomp) * P(incomp)

  5. Choose action = argmin expected loss
```

The asymmetric loss matrix penalizes "allow if incompatible" (100.0) far more than "reject if compatible" (6.0), making the system conservative by default. Every decision is recorded in the `EvidenceLedger` with full trace: timestamp, mode, prior, posterior, Bayes factor, and the evidence terms that drove it.

### Categorical Data

`DType::Categorical` is a reportable dtype identity, but the physical storage is an `Int64` code column with a parallel `CategoricalMetadata` record. This dual-layer design preserves dtype reporting parity with pandas while avoiding per-element `Scalar::Categorical(...)` allocations on every match arm across the workspace:

```
Series {
    name: String,
    index: Index,
    column: Column,                  // dtype=Int64; `values` holds the integer codes (0, 1, 2, ...)
    categorical: Some(CategoricalMetadata {
        categories: Vec<Scalar>,     // ["low", "medium", "high"]
        ordered: bool,               // Whether categories have total ordering
    }),
    sparse: None,                    // Optional pandas-style sparse metadata (separate dtype layer)
}
```

The `.cat()` accessor provides pandas-compatible operations: `categories()`, `codes()`, `rename_categories()`, `add_categories()`, `remove_unused_categories()`, `set_categories()`, `as_ordered()`, `as_unordered()`, `to_values()`. Missing values use code `-1`.

### MultiIndex

Hierarchical indexing is represented as parallel label vectors (one per level):

```
MultiIndex {
    levels: Vec<Vec<IndexLabel>>,  // levels[0] = ["a","a","b","b"]
    names: Vec<Option<String>>,    //            levels[1] = [1, 2, 1, 2]
}
```

Constructors mirror pandas: `from_tuples`, `from_arrays`, `from_product` (Cartesian product). Operations: `get_level_values(level)`, `droplevel(level)` → `MultiIndexOrIndex`, `swaplevel(i,j)`, `reorder_levels(order)`, `to_flat_index(sep)`.

DataFrame integration now carries a logical row MultiIndex alongside the flat storage index. `set_index_multi(&["col1", "col2"], drop, sep)` attaches real row-axis MultiIndex metadata, tuple-key `.loc[("a", 1)]` / `.xs(...)` / `.get_loc(...)` dispatch through that metadata, multi-key `groupby([k1, k2]).sum()` emits row-MultiIndex output, and reshape / IO round-trips preserve the row axis across `reset_index`, `stack`, `unstack`, CSV, Excel, Parquet, Feather, IPC, and JSON paths. `to_multi_index(&["col1", "col2"])` remains available when you want a standalone extracted MultiIndex value.

### String Accessor

The `.str()` accessor provides 50+ string operations matching pandas `Series.str`:

| Category | Methods |
|----------|---------|
| Case | `lower`, `upper`, `capitalize`, `title`, `casefold`, `swapcase` |
| Whitespace | `strip`, `lstrip`, `rstrip`, `expandtabs` |
| Search | `contains`, `startswith`, `endswith`, `find`, `rfind`, `index_of`, `rindex_of` |
| Transform | `replace`, `slice`, `repeat`, `pad`, `zfill`, `center`, `ljust`, `rjust` |
| Split/Join | `split_get`, `split_count`, `join`, `partition`, `rpartition` |
| Predicates | `isdigit`, `isalpha`, `isalnum`, `isspace`, `islower`, `isupper`, `isnumeric`, `isdecimal`, `istitle` |
| Regex | `contains_regex`, `replace_regex`, `replace_regex_all`, `extract`, `count_matches`, `findall`, `fullmatch`, `match_regex` |
| Prefix/Suffix | `removeprefix`, `removesuffix` |
| Other | `len`, `get`, `wrap`, `normalize`, `cat`, `get_dummies` |

### Join Types

| Type | Behavior | Output Size |
|------|----------|-------------|
| Inner | Only rows with keys in both sides | ≤ min(left, right) |
| Left | All left rows; right fills missing with NaN | = left |
| Right | All right rows; left fills missing with NaN | = right |
| Outer | All rows from both sides | ≤ left + right |
| Cross | Cartesian product (no key matching) | = left × right |
| Asof (backward) | Last right row where `right_key ≤ left_key` | = left |
| Asof (forward) | First right row where `right_key ≥ left_key` | = left |
| Asof (nearest) | Closest right row by absolute distance | = left |

`merge_asof` is particularly useful for time-series data, e.g., joining trades with quotes at the nearest preceding timestamp.

### Window Operations

Full rolling, expanding, and exponentially-weighted window support on both Series and DataFrame:

```rust
// Rolling window (like df["col"].rolling(30).mean())
let ma_30 = series.rolling(30, None).mean()?;
let vol   = series.rolling(252, Some(20)).std()?;

// Expanding window (cumulative)
let cum_max = series.expanding(None).max()?;

// Exponentially weighted moving average
let ewma = series.ewm(Some(20.0), None).mean()?;

// Time-based resampling
let monthly = series.resample("M").sum()?;
```

| Window Type | Series Methods | DataFrame Methods |
|-------------|---------------|-------------------|
| **Rolling** | `sum`, `mean`, `min`, `max`, `std`, `var`, `count`, `median`, `quantile`, `apply` | `sum`, `mean`, `min`, `max`, `std`, `var`, `count`, `median`, `quantile` |
| **Expanding** | `sum`, `mean`, `min`, `max`, `std`, `var`, `median`, `apply` | `sum`, `mean`, `min`, `max`, `std`, `var`, `median` |
| **EWM** | `mean`, `std`, `var` | `mean`, `std`, `var` |
| **Resample** | `sum`, `mean`, `count`, `min`, `max`, `first`, `last` | `sum`, `mean`, `count`, `min`, `max`, `first`, `last` |

GroupBy also supports `rolling()` and `resample()` for within-group window operations.

### Reshaping

All major pandas reshaping operations are implemented:

```rust
// Long → Wide
let pivoted = df.pivot_table("revenue", "region", "product", "sum")?;

// Wide → Long
let melted = df.melt(&["id"], &["q1", "q2", "q3"], "quarter", "sales")?;

// Stack/Unstack (with composite key round-trip)
let stacked = df.stack()?;
let unstacked = stacked.unstack()?;

// Contingency tables
let ct = df.crosstab("gender", "department")?;
let ct_norm = df.crosstab_normalize("gender", "department", "all")?;

// One-hot encoding
let dummies = df.get_dummies(&["color", "size"])?;

// Cross-section selection
let row = df.xs("2024-01-15")?;
```

### DataFrame Output Formats

12 inline output methods for different consumption contexts (plus 11 file / bytes writers via the [`DataFrameIoExt`](crates/fp-io/src/lib.rs) extension trait — `to_parquet`, `to_parquet_bytes`, `to_csv_file`, `to_json_file`, `to_excel_file`, `to_excel_bytes`, `to_jsonl_file`, `to_feather_file`, `to_feather_bytes`, `to_sql`, `to_sql_with_options`):

| Method | pandas Equivalent | Format |
|--------|-------------------|--------|
| `to_csv(sep, include_index)` | `df.to_csv()` | Comma/tab-separated values |
| `to_json(orient)` | `df.to_json()` | JSON with 5 orients |
| `to_string_table(include_index)` | `df.to_string()` | Aligned ASCII table |
| `to_string_truncated(idx, rows, cols)` | `df.to_string(max_rows=)` | Truncated with head/tail + "..." |
| `to_html(include_index)` | `df.to_html()` | HTML `<table>` |
| `to_latex(include_index)` | `df.to_latex()` | LaTeX `tabular` |
| `to_markdown(include_index, tablefmt)` | `df.to_markdown()` | GitHub/pipe, grid, and plain markdown/table text output |
| `to_dict(orient)` | `df.to_dict()` | dict/list/records/index/split/tight |
| `to_series_dict()` | `df.to_dict('series')` | `BTreeMap<String, Series>` |
| `to_records()` | `df.to_records()` | Vec of row vectors |
| `to_numpy_2d()` | `df.to_numpy()` | `Vec<Vec<f64>>` |
| `Display` trait | `print(df)` | Column-aligned with shape footer |

### GroupBy: Complete Aggregation Matrix

14 aggregation functions available through string dispatch:

| Function | Returns | Notes |
|----------|---------|-------|
| `sum` | Float64 | Null-skipping |
| `mean` | Float64 | Null-skipping |
| `count` | Int64 | Non-null count |
| `min` | Same as input | Null-skipping |
| `max` | Same as input | Null-skipping |
| `std` | Float64 | Sample standard deviation (ddof=1) |
| `var` | Float64 | Sample variance (ddof=1) |
| `median` | Float64 | Middle value |
| `first` | Same as input | First non-null |
| `last` | Same as input | Last non-null |
| `prod` | Float64 | Product of values |
| `sem` | Float64 | Standard error of mean |
| `skew` | Float64 | Fisher's skewness |
| `kurt`/`kurtosis` | Float64 | Excess kurtosis |

Plus group-level operations: `cumsum`, `cumprod`, `cummax`, `cummin`, `rank`, `shift`, `diff`, `nth`, `head`, `tail`, `pct_change`, `value_counts`, `describe`, `get_group`, `cumcount`, `ngroup`, `pipe`, `ohlc`.

### Datetime Parsing and Accessors

`to_datetime()` auto-detects common formats and normalizes to ISO 8601:

| Input Format | Example | Auto-Detected? |
|--------------|---------|----------------|
| ISO 8601 date | `2024-01-15` | Yes |
| ISO 8601 datetime | `2024-01-15T10:30:00` | Yes |
| Space-separated | `2024-01-15 10:30:00` | Yes |
| Slash date | `2024/01/15` | Yes |
| US date (MM/DD/YYYY) | `01/15/2024` | Yes |
| Epoch seconds (Int64) | `1705312200` | Yes |
| Epoch milliseconds | `1705312200000` | Yes (auto-detected from magnitude > 10^11) |
| Custom format | `15-Jan-2024` | Via `to_datetime_with_format(s, Some("%d-%b-%Y"))` |

The `.dt` accessor provides 20+ component extraction methods:

```rust
let years   = series.dt().year()?;     // Extract year
let months  = series.dt().month()?;    // 1-12
let dow     = series.dt().dayofweek()?; // Mon=0..Sun=6
let quarter = series.dt().quarter()?;  // 1-4
let woy     = series.dt().weekofyear()?; // ISO week 1-53
let fmt     = series.dt().strftime("%Y-%m-%d")?; // Custom format
```

Timezone support: `tz_localize(tz)`, `tz_convert(tz)` with chrono-tz for IANA timezone names.

`to_timedelta()` parses duration strings with similar flexibility:

```rust
// All of these work:
// "02:30:45"          → HH:MM:SS
// "3 days 04:15:30"   → pandas-style timedelta
// "5 days"            → day-only
// "3 hours"           → natural language
// Int64(3661)         → seconds (→ "01:01:01")
```

### Describe: Statistical Summary

`DataFrame.describe()` generates the same 8-row statistical summary as pandas:

```
         price      volume
count     8.0         8.0
mean    183.9       862.5
std       3.2       261.1
min     140.3       500.0
25%     141.4       575.0
50%     185.8       850.0
75%     186.3      1075.0
max     187.3      1200.0
```

Supports custom percentiles (`describe_with_percentiles(&[0.1, 0.5, 0.9])`) and dtype-filtered describe (`describe_dtypes(&["number"], &[])` for numeric only, or include `"object"` for string columns which produce count/unique/top/freq).

### Correlation and Covariance

Pairwise correlation and covariance matrices:

```rust
let pearson  = df.corr()?;               // Pearson (default)
let spearman = df.corr_method("spearman")?; // Rank correlation
let kendall  = df.corr_method("kendall")?;  // Kendall tau
let cov_mat  = df.cov()?;               // Covariance matrix
let corr_w   = df.corrwith(&other_df)?;  // Column-wise correlation
```

Series-level: `series.corr(&other)`, `series.cov_with(&other)`, `series.autocorr(lag)`.

### Apply and Transform

Multiple ways to apply custom logic:

```rust
// Element-wise on each column
let transformed = df.applymap(|scalar| { /* transform */ })?;

// Row-wise with full row access
let result = df.apply_row(|row_values| { /* produce scalar */ })?;

// Shape-preserving transform (output same shape as input)
let normed = df.transform("zscore")?;

// Column assignment with closures (pandas df.assign(new_col=lambda df: ...))
let df2 = df.assign_fn(vec![
    ("ratio", Box::new(|df| {
        // Compute new column from existing DataFrame
        Ok(compute_ratio_column(df))
    })),
])?;

// Pipe for method chaining
let result = df.pipe(|d| d.query("x > 0"))?.pipe(|d| d.sort_values("x", true))?;
```

### Module-Level Functions

Functions matching pandas top-level API:

| Function | pandas Equivalent | Description |
|----------|-------------------|-------------|
| `concat_dataframes` | `pd.concat` | Concatenate along axis 0 or 1 with join modes |
| `concat_dataframes_with_keys` | `pd.concat(keys=)` | Hierarchical index labeling |
| `to_datetime` | `pd.to_datetime` | Parse dates from strings/epochs |
| `to_timedelta` | `pd.to_timedelta` | Parse durations from strings/seconds |
| `to_numeric` | `pd.to_numeric` | Coerce to numeric with NaN for failures |
| `cut` | `pd.cut` | Equal-width binning |
| `qcut` | `pd.qcut` | Quantile-based binning |
| `timedelta_total_seconds` | `Series.dt.total_seconds()` | Convert timedelta to seconds |

## Performance

Five optimization rounds with formal evidence:

| Round | Optimization | Speedup |
|-------|-------------|---------|
| Round 1 | Remove duplicate `run_fixture_operation` invocation inside `run_fixture` | Eliminates redundant per-fixture work |
| Round 2 | `align_union` borrowed-key HashMap | Eliminates index clones |
| Round 3 | GroupBy identity-alignment fast path | Skips reindex when indexes match |
| Round 4 | Dense Int64 aggregation path | O(1) array access, no HashMap |
| Round 5 | `has_duplicates` OnceLock memoization | **87% faster** on groupby benchmark |

### Measured Baselines (10K rows, debug profile)

| Operation | p50 | p95 | Notes |
|-----------|-----|-----|-------|
| Join (inner, 50% overlap) | 12ms | 19ms | HashMap-based equijoin |
| Join (outer, 50% overlap) | 27ms | 37ms | Union index construction |
| Boolean filter (5 cols) | 15ms | 20ms | Reindex all columns by mask |
| `head(100)` | 22us | 28us | O(1) slice, no copy |
| Scalar add (5 cols) | 2.5ms | 2.8ms | Per-column vectorized |
| Scalar comparison (5 cols) | 3.0ms | 4.1ms | Per-column with Bool output |

Performance baselines tracked for join (inner/left/right/outer), filter (boolean mask, head/tail), and DataFrame arithmetic at 10K-100K row scales. Benchmarks run via `cargo test -p fp-conformance --test perf_baselines -- --ignored --nocapture`.

## Optimization Catalog

FrankenPandas applies 15 named optimization techniques (AG-01 through AG-15) drawn from the alien-graveyard systems-pattern library. Each is independently toggled and proven via isomorphism tests; the table below highlights 10 of the 15 (the remaining AG-01, AG-04, AG-09, AG-12, AG-15 are documented in their respective module headers under `crates/`):

| ID | Technique | Where Applied | What It Does |
|----|-----------|---------------|--------------|
| AG-02 | Borrowed-key HashMap | fp-index `align_union`, fp-join build phase | Builds position maps using `&IndexLabel` references instead of cloning labels. Eliminates O(n) allocations in the join build phase. |
| AG-03 | Identity-cast skip | fp-types `cast_scalar_owned` | When source dtype already equals target dtype, returns the value without cloning. Saves one allocation per element in column coercion. |
| AG-05 | N-way leapfrog triejoin | fp-index `multi_way_align` | Computes the union of N indexes in a single O(n log n) sorted-merge pass instead of iterative pairwise O(n^2 log n). Used by `DataFrame::from_series`. |
| AG-06 | Arena-backed execution | fp-groupby, fp-join | Routes intermediate allocations through Bumpalo bump allocator. Single `malloc`, pointer bumps, bulk dealloc. Configurable budget (default 256 MB). |
| AG-07 | Vec-based column accumulation | fp-io CSV parser | Pre-allocates `Vec<Vec<Scalar>>` with capacity hints from input byte count. O(1) amortized per cell vs O(log c) BTreeMap insertion. |
| AG-08 | Source-index referencing | fp-groupby HashMap path | Stores `(source_row_index, accumulator)` instead of `(Scalar_clone, accumulator)`. Reconstructs group key labels at output time, avoiding per-group Scalar clones. |
| AG-10 | Typed-array vectorization | fp-columnar binary ops | Dispatches to `&[f64]` / `&[i64]` typed arrays instead of per-element `match Scalar`. Enables LLVM auto-vectorization to SIMD. |
| AG-11 | Fast-path alignment skip | fp-frame, fp-groupby | When both operands share the same index and have no duplicates, skips the alignment planning phase entirely. O(1) check via `std::sync::OnceLock` memoization. |
| AG-13 | Adaptive sort-order lookup | fp-index `position()` | Lazily detects whether an index is sorted ascending (Int64 or Utf8). Uses O(log n) binary search for sorted, O(n) scan for unsorted. Sort order cached in `std::sync::OnceLock`. |
| AG-14 | Alignment plan validation | fp-index | Debug-mode assertion that position vectors have consistent lengths and valid indices. Catches alignment bugs at the source. |

## DType System and Coercion Rules

The type hierarchy determines how values are promoted when columns with different types interact:

```
Null (bottom type; promotes to anything)
 │
 ├── Bool
 │     └── Int64
 │           └── Float64
 │
 └── Utf8 (incompatible with numeric branch)
```

**Coercion rules** (matching pandas exactly):

| Left | Right | Result | Example |
|------|-------|--------|---------|
| Null | Any | That type | `Null + Int64 → Int64` |
| Bool | Int64 | Int64 | `True + 3 → 4` |
| Bool | Float64 | Float64 | `True + 1.5 → 2.5` |
| Int64 | Float64 | Float64 | `3 + 1.5 → 4.5` |
| Utf8 | Int64 | **Error** | Incompatible; fails closed |
| Utf8 | Float64 | **Error** | Incompatible; fails closed |
| Same | Same | Same | Identity; no coercion needed (AG-03 fast path) |

The identity-cast optimization (AG-03) detects when source dtype already matches target dtype and skips the clone entirely, which matters when `cast_scalar_owned()` is called millions of times during column coercion.

`infer_dtype(values)` folds `common_dtype()` across all elements to find the narrowest type that fits. This is used during CSV/JSON parsing where cell types are inferred individually and then unified per-column.

## Null Propagation Semantics

FrankenPandas distinguishes three kinds of missing values, exactly matching pandas:

| Kind | Meaning | Created By | `is_missing()` |
|------|---------|------------|-----------------|
| `Null` | Generic absence | Missing CSV cells, JSON `null`, outer join mismatches | `true` |
| `NaN` | Float not-a-number | `0.0 / 0.0`, explicit `f64::NAN`, Float64 column nulls | `true` |
| `NaT` | Not-a-time | Failed datetime parse, timedelta overflow | `true` |

**Propagation rules:**
- Arithmetic with missing: `5 + NaN → NaN`, `NaN + NaN → NaN`
- Comparison with missing: `NaN == NaN → false` (IEEE 754), `NaN != NaN → true`
- But `semantic_eq(NaN, NaN) → true` for index dedup and testing
- Aggregation skips nulls by default: `nansum([1, NaN, 3]) → 4`
- GroupBy with `dropna=true` excludes null group keys

The `ValidityMask` makes null checking O(1) per element (single bit test) and O(n/64) for bulk operations (word-level popcount).

## NanOps: Null-Aware Aggregation Library

The `fp-types` crate provides 10 null-skipping aggregation primitives that underpin all statistical operations:

```rust
// All skip Null/NaN values automatically:
nansum(&values)       // → Scalar::Float64 (sum of non-missing)
nanmean(&values)      // → Scalar::Float64 (mean of non-missing)
nancount(&values)     // → Scalar::Int64 (count of non-missing)
nanmin(&values)       // → same type as input minimum
nanmax(&values)       // → same type as input maximum
nanmedian(&values)    // → Scalar::Float64 (middle value)
nanvar(&values, ddof) // → Scalar::Float64 (sample variance, ddof=1 default)
nanstd(&values, ddof) // → Scalar::Float64 (sample std dev)
nanprod(&values)      // → Scalar::Float64 (product of non-missing)
nannunique(&values)   // → Scalar::Int64 (count of unique non-missing)
```

These are the building blocks for `Series.sum()`, `DataFrame.mean()`, `GroupBy.std()`, `describe()`, and every other statistical method. The "skip nulls by default" behavior matches pandas' `skipna=True` default.

Empty inputs or all-null inputs return `NaN` for float aggregations and `0` for count, matching pandas exactly.

## Error Architecture

Every crate has its own typed error enum, all implementing `std::error::Error` + `Display`:

| Error Type | Crate | Key Variants |
|-----------|-------|--------------|
| `TypeError` | fp-types | `IncompatibleDtypes { left, right }` |
| `ColumnError` | fp-columnar | `LengthMismatch`, `DtypeMismatch`, `EmptyColumn` |
| `IndexError` | fp-index | `OutOfBounds { position, length }`, `LengthMismatch`, `InvalidAlignmentVectors` |
| `FrameError` | fp-frame | `LengthMismatch`, `DuplicateIndexUnsupported`, `CompatibilityRejected(String)` |
| `ExprError` | fp-expr | `ParseError(String)`, `UnknownColumn(String)`, `UnknownLocal(String)` |
| `JoinError` | fp-join | wraps `FrameError` + join-specific failures |
| `GroupByError` | fp-groupby | wraps `FrameError` + aggregation failures |
| `IoError` | fp-io | `MissingHeaders`, `MissingIndexColumn`, `Csv(...)`, `Json(...)`, `Parquet(...)`, `Excel(...)`, `Arrow(...)`, `Sql(...)` |

All error types are re-exported through the `frankenpandas` facade crate, so users can pattern-match without importing internal crates:

```rust
use frankenpandas::{IoError, FrameError, ExprError};

match result {
    Err(IoError::MissingHeaders) => eprintln!("CSV has no headers"),
    Err(IoError::Sql(msg)) => eprintln!("SQL error: {msg}"),
    _ => {}
}
```

## DataFrame Constructors

15+ ways to create a DataFrame, matching every pandas construction pattern:

| Constructor | pandas Equivalent | Example |
|-------------|-------------------|---------|
| `from_dict(&[col_order], data)` | `pd.DataFrame({"a": [1,2], "b": [3,4]})` | Column-oriented dict |
| `from_dict_with_index(data, labels)` | `pd.DataFrame(data, index=[...])` | With custom index |
| `from_dict_mixed(data)` | `pd.DataFrame({"a": [1], "b": ["x"]})` | Mixed types per column |
| `from_series(vec![s1, s2])` | `pd.DataFrame({"a": s1, "b": s2})` | From Series with alignment |
| `from_records(records, columns)` | `pd.DataFrame.from_records(...)` | Row-oriented records |
| `from_tuples(rows, columns)` | `pd.DataFrame([(1,"a"), (2,"b")])` | Tuple rows |
| `from_tuples_with_index(rows, cols, idx)` | Same with custom index | Tuples + index labels |
| `from_csv(text, sep)` | `pd.read_csv(StringIO(text))` | Inline CSV string |
| `from_dict_index(data)` | `pd.DataFrame.from_dict(data, orient='index')` | Row-keyed dict |
| `from_dict_index_columns(data, cols)` | Same with column names | Row-keyed + col names |
| `DataFrame::new(index, columns)` | Low-level construction | Direct index + BTreeMap |
| `DataFrame::new_with_row_multiindex(index, row_multiindex, columns)` | `pd.DataFrame(..., index=pd.MultiIndex(...))` foundation | Flat storage index + logical row MultiIndex metadata |

When constructing from multiple Series with different indexes, `from_series` automatically performs N-way index alignment (AG-05 leapfrog triejoin) to produce a DataFrame with the union of all index labels.

`DataFrame::row_index()` returns the logical row axis as `MultiIndexOrIndex`, and `DataFrame::row_multiindex()` exposes the optional row-side `MultiIndex` metadata directly. `DataFrame::index()` remains the flat storage fallback used internally for compatibility with existing code paths.

## Merge: Advanced Options

Beyond the basic join types, the merge system supports pandas' full merge parameter set:

```rust
// Merge with validation (like pandas validate='one_to_one')
let merged = df1.merge_with_options(&df2, &["key"], JoinType::Inner,
    MergeOptions {
        validate_mode: Some(MergeValidateMode::OneToOne),
        ..Default::default()
    })?;

// Merge with indicator column (like pandas indicator=True)
// Adds a column showing "left_only", "right_only", or "both"
let merged = df1.merge_with_options(&df2, &["key"], JoinType::Outer,
    MergeOptions {
        indicator_name: Some("_merge".to_owned()),
        ..Default::default()
    })?;

// Merge with custom suffixes for overlapping columns
let merged = df1.merge_with_options(&df2, &["key"], JoinType::Inner,
    MergeOptions {
        suffixes: Some([Some("_left".to_owned()), Some("_right".to_owned())]),
        ..Default::default()
    })?;
```

**Validation modes** catch data quality issues at merge time:

| Mode | Constraint | Fails When |
|------|-----------|------------|
| `OneToOne` | Each key appears once in both sides | Duplicates in either side |
| `OneToMany` | Keys unique in left, may repeat in right | Duplicates in left |
| `ManyToOne` | Keys may repeat in left, unique in right | Duplicates in right |
| `ManyToMany` | No constraint (default) | Never |

## Testing

**3,200+ tests** across the workspace:

| Category | Count | What It Covers |
|----------|-------|----------------|
| fp-frame unit tests | 1,433 | DataFrame, Series, Categorical, MultiIndex integration |
| fp-io tests | 429 | 8 IO formats, adversarial inputs, round-trip correctness, full SQL trait surface (introspection / chunking / per-column dtype overrides) |
| fp-columnar tests | 281 | Column / ColumnData / ValidityMask / arithmetic / comparison / sparse encoding |
| fp-index tests | 199 | Index alignment, MultiIndex, duplicate detection |
| fp-types tests | 155 | DType / Scalar / Timedelta / Timestamp / nan-aware aggregations |
| fp-join tests | 70 | Inner/Left/Right/Outer/Cross/Asof joins, merge_asof edge cases |
| fp-groupby tests | 65 | Per-aggregation kernels (sum / mean / std / median / nunique / ...), HyperLogLog approximate counting |
| fp-expr tests | 52 | Expression parsing, eval/query, @local variables, MaterializedView caching |
| fp-runtime tests | 30 | RuntimePolicy, EvidenceLedger, ConformalGuard, RaptorQ envelopes |
| fp-frankentui tests | 22 | TUI snapshot model, E2E scenario harness |
| fp-conformance tests | 557 | Differential conformance against pandas oracle (live + fixture replay) |
| Property-based (proptest) | 75 | DType coercion, CSV/JSON/SQL/Excel/Feather round-trip, ValidityMask algebra, DataFrame arithmetic invariants |
| End-to-end pipeline | 11 | Full workflow: CSV read → query → groupby → merge → sort → export |
| Performance baselines | 13 | Join/filter/arithmetic latency measurement |

### Conformance Gate

```bash
./scripts/phase2c_gate_check.sh
```

Regenerates conformance packet artifacts and fails closed if any parity report or gate is not green. 430+ packet suites spanning 1250 fixtures cover alignment, join, groupby, concat, filter, CSV, dtype, null semantics, resample, rolling, groupby rolling/resample, datetime accessors, string accessors, and more. The vast majority pass under `cargo test -p fp-conformance` (557+ passing); a small number of known mismatch packets remain on the backlog (currently 3 packet filter cases on time-zone parse_dates, axis-1 concat, and JSON records orient, plus the FP-P2C-001 series_add packets on union sort + Int64→Float64 promotion). Each is documented with full root-cause analysis in [`crates/fp-conformance/DISCREPANCIES.md`](crates/fp-conformance/DISCREPANCIES.md) (DISC-011 through DISC-014), so users hitting these failures find the explanation and the WILL-FIX status without having to re-derive the divergence.

## Missing Data Handling

Complete pandas-compatible toolkit for detecting, filling, and removing missing values:

### Detection

```rust
let nulls = series.isna()?;        // Bool Series: true where null/NaN
let valid = series.notna()?;       // Bool Series: true where valid
let count = series.count();        // Count of non-missing values
let has   = series.hasnans();      // true if any missing values exist
```

DataFrame-level: `df.isna()`, `df.notna()`, `df.isnull()`, `df.notnull()` return DataFrames of Booleans. `first_valid_index()` and `last_valid_index()` scan for the first/last non-null row.

### Filling

```rust
// Fill with constant
let filled = series.fillna(&Scalar::Float64(0.0))?;

// Forward fill (propagate last valid value)
let ffilled = series.ffill(None)?;      // No limit
let ffilled = series.ffill(Some(3))?;   // Fill at most 3 consecutive NaN

// Backward fill (propagate next valid value)
let bfilled = series.bfill(Some(2))?;

// Linear interpolation
let interp = series.interpolate()?;

// Combine two DataFrames, filling nulls from `other`
let combined = df.combine_first(&other)?;  // Like pd.DataFrame.combine_first

// Update in place from another DataFrame
let updated = df.update(&other)?;  // Non-null values in `other` overwrite `df`
```

DataFrame-level: `df.fillna(&scalar)`, `df.ffill(limit)`, `df.bfill(limit)`, `df.interpolate()`, `df.fillna_method("ffill")` / `df.fillna_method("bfill")`.

### Dropping

```rust
// Drop rows with any null (default)
let clean = df.dropna()?;

// Drop rows where ALL values are null
let clean = df.dropna_with_options(DropNaHow::All, None)?;

// Drop rows with nulls in specific columns only
let clean = df.dropna_with_options(DropNaHow::Any, Some(&["price".into(), "volume".into()]))?;

// Drop rows with fewer than N non-null values (thresh)
let clean = df.dropna_with_thresh(3)?;

// Drop COLUMNS with nulls (axis=1)
let clean = df.dropna_columns()?;
```

## Type Coercion and Conversion

```rust
// Cast Series to a specific dtype
let float_col = int_series.astype(DType::Float64)?;

// Cast DataFrame column
let df2 = df.astype_column("price", DType::Float64)?;

// Cast multiple columns at once
let df2 = df.astype_columns(&[("id", DType::Utf8), ("score", DType::Float64)])?;

// Auto-infer best dtypes (Utf8 → Int64/Float64 where possible)
let df2 = df.convert_dtypes()?;
let df2 = df.infer_objects()?;  // Same idea, pandas-compatible name

// Coerce to numeric with NaN for failures
let numeric = to_numeric(&string_series)?;
```

The `common_dtype()` function determines the result type when combining two columns. It follows pandas' promotion rules exactly: `Bool + Int64 → Int64`, `Int64 + Float64 → Float64`, `Utf8 + numeric → Error`.

## Element-Wise Operations

### Arithmetic

```rust
// Scalar operations
let doubled = df.mul_scalar(2.0)?;
let offset  = df.add_scalar(100.0)?;
let pct     = df.div_scalar(total)?;
let squared = df.pow_scalar(2.0)?;
let modulo  = df.mod_scalar(10.0)?;
let floored = df.floordiv_scalar(3.0)?;

// DataFrame-to-DataFrame (with automatic index alignment)
let diff  = df1.sub_df(&df2)?;         // Aligned subtraction
let ratio = df1.div_df(&df2)?;         // Aligned division
let product = df1.mul_df(&df2)?;       // Aligned multiplication

// With fill_value for missing alignment positions
let sum = df1.add_df_fill(&df2, 0.0)?;  // Fill missing with 0 before adding
```

### Cumulative and Sequential

```rust
let csum  = df.cumsum()?;      // Running sum
let cprod = df.cumprod()?;     // Running product
let cmax  = df.cummax()?;      // Running maximum
let cmin  = df.cummin()?;      // Running minimum
let delta = df.diff(1)?;       // First difference (n periods)
let moved = df.shift(1)?;      // Shift values by n periods
let pct   = df.pct_change()?;  // Percentage change
```

### Clipping and Rounding

```rust
let clipped = df.clip(0.0, 100.0)?;    // Clip to [0, 100]
let lower   = df.clip_lower(0.0)?;     // Floor at 0
let upper   = df.clip_upper(100.0)?;   // Cap at 100
let rounded = df.round(2)?;            // Round to 2 decimal places
let absolute = df.abs()?;              // Absolute value
```

### Replacement

```rust
// Replace specific values
let cleaned = df.replace(&Scalar::Int64(-999), &Scalar::Null(NullKind::NaN))?;

// Series: regex replace
let fixed = series.str().replace_regex(r"\d{3}-\d{4}", "***-****")?;

// Series: map with replacement dictionary
let mapped = series.map_with_na_action(&mapping, true)?;  // na_action=ignore

// Series: conditional assignment
let graded = scores.case_when(&[
    (scores.ge(&Scalar::Int64(90))?, Series::constant("A", n)?),
    (scores.ge(&Scalar::Int64(80))?, Series::constant("B", n)?),
])?;
```

## Advanced Selection Methods

Beyond basic `loc`/`iloc`, FrankenPandas provides the full pandas selection toolkit:

```rust
// Top-N and Bottom-N rows by column value
let top5 = df.nlargest(5, "revenue")?;
let bot3 = df.nsmallest(3, "price")?;
// With keep parameter: 'first' (default), 'last', 'all'
let top5 = df.nlargest_keep(5, "revenue", "all")?;

// Find label of min/max value
let worst_day = series.idxmin()?;   // → IndexLabel of minimum
let best_day  = series.idxmax()?;   // → IndexLabel of maximum

// Value counts (like Series.value_counts())
let counts = series.value_counts()?;
// With full options: normalize, sort, ascending, dropna
let pcts = series.value_counts_with_options(true, true, false, true)?;

// Membership testing
let mask = series.isin(&[Scalar::Utf8("A".into()), Scalar::Utf8("B".into())])?;
let in_range = series.between(&Scalar::Int64(10), &Scalar::Int64(20))?;

// Index-based position lookup
let pos = series.searchsorted(&Scalar::Float64(42.0), "left")?;
let (codes, uniques) = series.factorize()?;  // Encode as integers

// Select columns by dtype
let numeric_only = df.select_dtypes(&[DType::Int64, DType::Float64], &[])?;
let non_numeric = df.select_dtypes(&[], &[DType::Int64, DType::Float64])?;

// Flexible label-based filtering
let subset = df.filter_labels(Some(&["price", "volume"]), None, None, 1)?; // axis=1
let regex_match = df.filter_labels(None, None, Some("^rev"), 1)?;  // Regex on col names

// Reindex to new labels (fill missing with NaN)
let reindexed = series.reindex(new_labels)?;
let trimmed = series.truncate(Some(&start_label), Some(&end_label))?;

// Column manipulation
let (popped_series, remaining_df) = df.pop("temp_col")?;       // Remove and return
let with_new = df.insert(2, "computed", new_column)?;            // Positional insert
let renamed = df.add_prefix("raw_")?;                           // Prefix all columns
let renamed = df.add_suffix("_v2")?;                             // Suffix all columns
```

## DataFrame Introspection

```rust
let shape = df.shape();              // (nrows, ncols)
let dtypes = df.dtypes();            // Vec<(column_name, DType)>
let info = df.info();                // String summary (like df.info() in pandas)
let mem = df.memory_usage()?;        // Per-column byte estimates as Series
let ndim = df.ndim();                // Always 2 for DataFrame
let axes = df.axes();                // (index, column_names)
let is_empty = df.is_empty();        // True if zero rows

// Deep equality (structural + value comparison)
let same = df1.equals(&df2);         // true if identical structure and values

// Element-wise diff between two DataFrames
let changes = df1.compare(&df2)?;    // Shows only positions that differ

// Squeeze single-column/single-row DataFrames
let series = single_col_df.squeeze(1)?;   // DataFrame → Series
let scalar = single_cell_df.squeeze(0)?;  // DataFrame → Scalar

// Scalar access
let val = series.iat(0)?;           // By position (like .iat[0])
let val = series.at(&label)?;       // By label (like .at[label])

// Lookup specific (row, col) pairs
let values = df.lookup(&row_labels, &col_names)?;
```

## SeriesGroupBy

Series-level groupby is separate from DataFrame groupby and provides a lightweight API for single-column aggregation:

```rust
// Group one Series by another
let by_region = revenue_series.groupby(&region_series)?;

// Aggregate
let sums  = by_region.sum()?;
let means = by_region.mean()?;
let stds  = by_region.std()?;
let meds  = by_region.median()?;
let prods = by_region.prod()?;

// Multiple aggregations at once
let multi = by_region.agg(&["sum", "mean", "count"])?;  // Returns DataFrame
```

`SeriesGroupBy` supports: `sum`, `mean`, `count`, `min`, `max`, `std`, `var`, `median`, `first`, `last`, `prod`, `sem`, `skew`, `kurtosis`, `agg` (multi-function), and `value_counts`.

## Sorting

```rust
// Sort by column values
let sorted = df.sort_values("price", true)?;          // ascending=true
let sorted = df.sort_values("price", false)?;         // descending

// Control NaN position
let sorted = series.sort_values_na(true, "first")?;   // NaN at top
let sorted = series.sort_values_na(true, "last")?;    // NaN at bottom (default)

// Sort by index labels
let sorted = df.sort_index(true)?;                    // ascending
let sorted = df.sort_index(false)?;                   // descending
```

## Concat: Full Options

```rust
// Axis 0 (stack rows, default)
let stacked = concat_dataframes(&[&df1, &df2])?;

// Axis 1 (add columns side-by-side, outer join on index)
let wide = concat_dataframes_with_axis(&[&df1, &df2], 1)?;

// Axis 1 with inner join (only shared index labels)
let inner = concat_dataframes_with_axis_join(&[&df1, &df2], 1, ConcatJoin::Inner)?;

// Axis 0 with inner join (only shared columns)
let shared = concat_dataframes_with_axis_join(&[&df1, &df2], 0, ConcatJoin::Inner)?;

// With hierarchical keys
let labeled = concat_dataframes_with_keys(&[&df1, &df2], &["train", "test"])?;

// Ignore original indexes (reindex to 0..n)
let clean = concat_dataframes_with_ignore_index(&[&df1, &df2])?;
```

## Pivot Tables: Full Options

```rust
// Basic pivot table
let pt = df.pivot_table("revenue", "region", "product", "sum")?;

// Multiple values columns
let pt = df.pivot_table_multi_values(&["revenue", "quantity"], "region", "product", "sum")?;

// With margins (subtotals row/column)
let pt = df.pivot_table_with_margins("revenue", "region", "product", "sum")?;

// Custom margins label
let pt = df.pivot_table_with_margins_name("revenue", "region", "product", "sum", "Grand Total")?;

// Fill NaN in pivot output
let pt = df.pivot_table_fill("revenue", "region", "product", "sum", 0.0)?;

// Multiple aggregation functions
let pt = df.pivot_table_multi_agg("revenue", "region", "product", &["sum", "mean", "count"])?;
```

## Time-Series Operations

```rust
// Select rows at a specific time
let noon = df.at_time("12:00:00")?;

// Select rows within a time range
let morning = df.between_time("09:00:00", "12:00:00")?;

// Datetime component extraction (full list)
let components = series.dt();
components.year()?;            components.month()?;
components.day()?;             components.hour()?;
components.minute()?;          components.second()?;
components.dayofweek()?;       components.dayofyear()?;
components.quarter()?;         components.weekofyear()?;
components.is_month_start()?;  components.is_month_end()?;
components.is_quarter_start()?; components.is_quarter_end()?;
components.strftime("%Y-%m-%d %H:%M")?;

// Timezone operations
let localized = series.dt().tz_localize(Some("America/New_York"))?;
let converted = series.dt().tz_convert(Some("UTC"))?;
```

## Column Manipulation

```rust
// Rename columns
let renamed = df.rename_with(|name| format!("col_{name}"))?;
let prefixed = df.add_prefix("input_")?;
let suffixed = df.add_suffix("_raw")?;

// Assign new column (value vector)
let df2 = df.assign_column("computed", computed_values)?;

// Assign with closure (sees current DataFrame state)
let df2 = df.assign_fn(vec![
    ("ratio", Box::new(|df: &DataFrame| {
        // Compute from existing columns
        let a = df.column("revenue").unwrap();
        let b = df.column("cost").unwrap();
        // ... return Column
        Ok(result_column)
    })),
])?;

// Reorder columns
let reordered = df.select_columns(&["id", "name", "value"])?;
```

## Recipes

### Financial Data Pipeline

```rust
use frankenpandas::prelude::*;

// Load trade data
let trades = read_csv_str(
    "date,ticker,price,volume\n\
     2024-01-15,AAPL,185.50,1000\n\
     2024-01-15,GOOG,140.25,500\n\
     2024-01-16,AAPL,186.00,1200\n\
     2024-01-16,GOOG,141.00,800"
)?;

// Parse dates
let date_series = Series::new("date", trades.index().clone(),
    trades.column("date").unwrap().clone())?;
let parsed_dates = to_datetime(&date_series)?;

// Daily VWAP per ticker
let vwap = trades.groupby(&["ticker"])?.agg_named(&[
    ("total_value", "price", "sum"),   // Simplified; real VWAP needs price*vol
    ("total_vol", "volume", "sum"),
    ("trade_count", "volume", "count"),
])?;

// Export for downstream consumption
write_jsonl(&vwap, Path::new("daily_vwap.jsonl"))?;
```

### Merge-Asof for Time Series Alignment

```rust
// Join trades with quotes at the nearest preceding timestamp
let result = merge_asof(
    &trades, &quotes, "timestamp", AsofDirection::Backward
)?;
// Each trade row now has the most recent quote as of that trade time
```

### Categorical Analysis

```rust
// Create categorical with explicit ordering
let ratings = Series::from_categorical(
    "satisfaction",
    vec![
        Scalar::Utf8("good".into()),
        Scalar::Utf8("poor".into()),
        Scalar::Utf8("excellent".into()),
        Scalar::Utf8("good".into()),
    ],
    true, // ordered
)?;

// Access category operations
let cat = ratings.cat().unwrap();
println!("Categories: {:?}", cat.categories());  // [good, poor, excellent]
println!("Codes: {:?}", cat.codes()?.values());   // [0, 1, 2, 0]

// Rename categories
let renamed = cat.rename_categories(vec![
    Scalar::Utf8("Good".into()),
    Scalar::Utf8("Poor".into()),
    Scalar::Utf8("Excellent".into()),
])?;

// Materialize back to values
let values = renamed.cat().unwrap().to_values()?;
```

### MultiIndex Operations

```rust
// Create from product (Cartesian)
let mi = MultiIndex::from_product(vec![
    vec!["east".into(), "west".into()],
    vec![2023i64.into(), 2024i64.into()],
])?
.set_names(vec![Some("region".into()), Some("year".into())]);

// Extract a level
let regions = mi.get_level_values(0)?;

// Flatten to single index
let flat = mi.to_flat_index("_");  // "east_2023", "east_2024", ...

// From DataFrame columns
let mi = df.to_multi_index(&["region", "year"])?;
```

### Expression-Driven Analysis

```rust
use std::collections::BTreeMap;

// Compute new columns with eval
let profit = df.eval("revenue - cost")?;

// Filter with compound conditions
let hot_deals = df.query("price < 50 and rating > 4.0")?;

// Use local variables in expressions
let locals = BTreeMap::from([
    ("threshold".to_owned(), Scalar::Float64(100.0)),
]);
let above = df.query_with_locals("value > @threshold", &locals)?;
```

## Conformance Testing in Depth

The conformance system is a differential testing framework that verifies FrankenPandas output against the actual pandas library:

```
                  ┌───────────────┐
                  │ Fixture JSON   │  ← Input DataFrame + operation
                  └───────┬───────┘
                          │
              ┌───────────┴───────────┐
              ▼                       ▼
    ┌───────────────────┐   ┌─────────────────────┐
    │ FrankenPandas      │   │ pandas Oracle        │
    │ (Rust execution)   │   │ (Python subprocess)  │
    └─────────┬─────────┘   └──────────┬──────────┘
              │                        │
              ▼                        ▼
    ┌──────────────────────────────────────────┐
    │          Parity Comparison                │
    │  dtype match?  value match?  order?       │
    └────────────────────┬─────────────────────┘
                         │
              ┌──────────┴──────────┐
              ▼                     ▼
    ┌────────────────┐   ┌────────────────┐
    │ parity_report   │   │ parity_gate    │
    │ .json           │   │ _result.json   │
    └────────────────┘   └────────────────┘
```

**Packet families** cover: series alignment (FP-P2C-001–003), join semantics (FP-P2C-004), groupby aggregates (FP-P2C-005, 011), concat (FP-P2C-006), null/NaN ops (FP-P2C-007), CSV round-trip (FP-P2C-008), dtype invariants (FP-P2C-009), filter/selection (FP-P2C-010), plus 30+ DataFrame-level packets (FP-P2D-014 through FP-P2D-055) covering merge, concat axis options, head/tail, loc/iloc, sort, constructor variants, and more.

Every parity report gets a **RaptorQ repair-symbol sidecar** for bit-rot detection. The drift history ledger (`artifacts/phase2c/drift_history.jsonl`) tracks parity trends over time.

## Troubleshooting

| Problem | Cause | Fix |
|---------|-------|-----|
| `cargo build` fails with "edition 2024" error | Requires Rust nightly | Install via `rustup toolchain install nightly` and check `rust-toolchain.toml` |
| `Column::from_values` returns `IncompatibleDtypes` | Mixed Utf8 + numeric values in one column | Ensure homogeneous types, or use `to_numeric()` first |
| `query("col > 5")` returns error | Column name not found in DataFrame | Verify column exists with `df.column_names()` |
| CSV round-trip changes Float64 to Int64 | `1.0` written as `"1"`, parsed back as Int64 | Use `dtype` parameter in `CsvReadOptions` to force types |
| SQL write fails with "table already exists" | Default `SqlIfExists::Fail` policy | Use `SqlIfExists::Replace` or `SqlIfExists::Append` |
| Excel round-trip loses integer precision for large values | Excel stores all numbers as f64 | Values within i64 range with zero fraction are recovered as Int64 |
| JSONL reader drops columns | Columns only in later rows were missed | Fixed: reader now unions all keys across all rows |
| `to_datetime` returns NaT | Unrecognized date format | Use `to_datetime_with_format(series, Some("%d/%m/%Y"))` with explicit format |

## Limitations

| Limitation | Status | Workaround |
|-----------|--------|------------|
| No native Datetime dtype | Datetimes stored as ISO 8601 Utf8 strings | Use `to_datetime()` for normalization |
| Categorical metadata not propagated through arithmetic | By design (matches pandas) | Use `.cat().to_values()` to materialize |
| No HDF5, Clipboard, or HTML IO | System-library dependencies | Use Feather (faster) or Parquet instead |
| SQL IO has one built-in backend | `read_sql` / `write_sql` are generic over `SqlConnection`, and `rusqlite::Connection` implements it behind the default `sql-sqlite` feature. The trait surface is feature-complete (introspection via `SqlInspector` + `reflect_table` / `reflect_all_tables`, `SqlReadOptions` / `SqlWriteOptions` matching pandas keyword args including `chunksize`, `coerce_float`, `dtype`, `parse_dates`, `index_col`, `columns`, `schema`, `method`). Concrete PostgreSQL/MySQL/MS SQL/Oracle adapters are not yet bound — placeholder feature flags `sql-postgresql` / `sql-mysql` exist on fp-io for the future Phase 2 backend integrations. | Use SQLite via `rusqlite::Connection::open[_in_memory]`, or implement `SqlConnection` for another backend while native adapters land |
| Single-threaded execution | No parallel execution yet | Profile-proven fast paths compensate |
| No plotting | Requires graphics library | Export to pandas/matplotlib for visualization |

## FAQ

**Q: How compatible is this with pandas?**
A: We target absolute API parity. The same method names, same parameter names, same edge-case behavior. Differential conformance tests verify against the actual pandas oracle. We're in early development so not every method is implemented yet, but the architecture is designed for full coverage.

**Q: Why not just use Polars?**
A: Polars is excellent but has a fundamentally different API (lazy evaluation, no index alignment, different method names). FrankenPandas targets Rust teams that want pandas semantics rather than Polars' query-planner style — identical method names, identical edge-case behavior. (Python drop-in requires the planned PyO3 bindings; today FrankenPandas is a Rust library.)

**Q: Is `unsafe` code used anywhere?**
A: No. Every crate in the workspace uses `#![forbid(unsafe_code)]`. Memory safety comes from the Rust type system.

**Q: How do I use this from Python?**
A: PyO3 bindings are a planned future step. Currently this is a pure Rust library.

**Q: What's the `EvidenceLedger`?**
A: Every alignment decision, dtype coercion, and policy override is logged with Bayesian confidence scores. This creates an auditable trail of exactly how your data was transformed. pandas makes these same decisions silently with no record.

**Q: What's the performance like?**
A: Five formal optimization rounds with measured evidence. The groupby path alone saw an 87% speedup through memoization and dense aggregation paths. All optimizations include isomorphism proofs showing behavior is unchanged.

**Q: How is the GroupBy implemented?**
A: Three automatic execution paths. Dense Int64 keys (range ≤ 65,536) use O(1) array indexing. Medium cardinality uses Bumpalo arena allocation (single malloc, zero fragmentation). Everything else falls back to a HashMap with source-index referencing to avoid per-group clones. All three produce identical results, verified by property-based tests.

**Q: What does "clean-room" mean?**
A: We never read, reference, or copy from the pandas source code. We study pandas' *behavior* (input → output contracts, edge cases, dtype rules) via the conformance oracle, then implement from first principles in Rust. This avoids any license contamination and often produces better implementations.

**Q: How do you handle NaN vs None vs NaT?**
A: Three distinct null kinds: `NullKind::Null` (generic missing), `NullKind::NaN` (float not-a-number), `NullKind::NaT` (not-a-time). `Float64(NaN)` also counts as missing. `is_missing()` returns true for all of these. `semantic_eq()` treats `NaN == NaN` as true (unlike IEEE 754), matching pandas behavior.

**Q: What's the memory overhead vs pandas?**
A: Comparable for numeric data. `ValidityMask` uses 1 bit per element (vs pandas' 8-byte nullable dtype). `Column` stores typed arrays (`Vec<f64>`, `Vec<i64>`) instead of object arrays. String data uses `Vec<String>` (heap-allocated per element, same as pandas object dtype). Arena-backed GroupBy/Join operations avoid per-group heap fragmentation.

**Q: Can I use this for production ETL pipelines?**
A: The core DataFrame, IO, and GroupBy/Join operations are solid and well-tested (3,200+ tests including adversarial inputs and property-based fuzzing). This is pre-1.0 software, so API stability is not yet guaranteed, but the correctness bar is high.

## Selection and Indexing

Full pandas-style selection API:

```rust
// Label-based (like df.loc[])
let row = df.loc("row_label")?;           // Single row by label
let subset = df.loc_rows(&["a", "b"])?;   // Multiple rows

// Position-based (like df.iloc[])
let row = df.iloc(0)?;                     // First row
let last = df.iloc(-1)?;                   // Last row (negative indexing)
let slice = df.head(10);                   // First 10 rows
let tail = df.tail(5);                     // Last 5 (supports negative n)

// Column selection
let col = df.column("price")?;             // Single column as &Column
let subset = df.select_columns(&["price", "volume"])?;
let numeric = df.select_dtypes(&["int64", "float64"], &[])?;

// Boolean masking
let mask = df.query("price > 100")?;
let filtered = df.filter_rows(&bool_series)?;

// Conditional replacement
let filled = df.where_mask_df(&cond_df, &other_df)?;
let masked = df.mask_df(&cond_df, &other_df)?;

// Index operations
let reindexed = df.set_index("date", true)?;  // Column → index
let reset = df.reset_index(false)?;            // Index → column
let sorted = df.sort_index(true)?;             // Sort by index
let deduped = df.drop_duplicates()?;           // Remove duplicate rows
```

## Serialization and Interoperability

All core types are fully serializable via serde:

```rust
// Every type derives Serialize + Deserialize:
// Scalar, DType, NullKind, IndexLabel, Index, MultiIndex,
// Series, DataFrame, CategoricalMetadata, Column, ValidityMask

// JSON serialization
let json = serde_json::to_string(&scalar)?;      // Tagged enum: {"kind":"int64","value":42}
let back: Scalar = serde_json::from_str(&json)?;  // Round-trips perfectly

// Binary serialization (via bincode, messagepack, etc.)
let bytes = bincode::serialize(&dataframe)?;
let back: DataFrame = bincode::deserialize(&bytes)?;
```

The `Scalar` enum uses serde's tagged representation (`#[serde(tag = "kind", content = "value")]`) for human-readable JSON while remaining efficient for binary formats. `ValidityMask` serializes as a `Vec<bool>` for JSON compatibility but uses bitpacked `Vec<u64>` in memory.

**Arrow interop:** DataFrame ↔ Arrow RecordBatch conversion is built in (used by Parquet and Feather IO). This means FrankenPandas data can be zero-copy shared with any Arrow-compatible system (DuckDB, DataFusion, Spark via Arrow Flight).

## Adversarial and Property-Based Testing

Beyond unit tests, FrankenPandas employs two advanced testing strategies:

### Property-Based Tests (proptest)

75 properties that must hold for ALL randomly-generated inputs:

| Property Category | Examples | Cases |
|-------------------|----------|-------|
| DType coercion | `common_dtype` is symmetric, reflexive, transitive | 500 |
| Scalar cast | Identity cast preserves value, missing stays missing | 500 |
| Index alignment | Union contains all labels, position vectors correct length | 500 |
| Series arithmetic | Self-add doubles values, no panics in hardened mode | 200 |
| Join invariants | Inner ⊆ Left ⊆ Outer, output lengths consistent | 200 |
| GroupBy | Groups bounded by input rows, arena ≡ global allocator | 200 |
| ValidityMask | De Morgan's law, NOT involution, AND commutativity | 300 |
| CSV round-trip | Shape preserved, column names preserved, Int64 exact | 100 |
| JSON round-trip | Shape preserved across Records/Columns/Split/Values | 50 |
| SQL round-trip | Shape and values preserved through SQLite | 30 |
| Excel round-trip | Int64 values recovered from f64 through xlsx | 30 |
| Feather round-trip | Shape, names, and values exact through Arrow IPC | 50 |
| DataFrame arithmetic | add_scalar preserves shape, add(0) ≡ identity | 100 |
| Comparison ops | eq XOR ne = true for non-NaN values | 100 |

### Adversarial Input Tests

16 tests targeting parser edge cases:

- **CSV:** 200K-character field, 1000 columns, embedded newlines in quotes, multi-byte UTF-8 (Japanese, Cyrillic, emoji), header-only files, no trailing newline, empty rows
- **JSON:** Deeply nested objects, `i64::MAX`/`i64::MIN` boundary values, near-`f64::MAX` floats, empty `Records` arrays, empty `Columns` objects
- **SQL:** 10,000-row batch insert, column names with spaces (quoted identifier handling), column names with embedded double-quotes, SQL injection rejection (Bobby Tables pattern)

## Duplicate Handling

Pandas-compatible `keep` parameter for duplicate detection:

```rust
// Mark duplicates (like df.duplicated(keep='first'))
let mask = df.duplicated()?;  // First occurrence = false, subsequent = true

// Drop duplicates
let unique = df.drop_duplicates()?;

// Series-level with keep parameter
let deduped = series.drop_duplicates()?;

// Index-level
let has_dups = index.has_duplicates();  // O(1) after first call (OnceLock)
let unique_idx = index.drop_duplicates(DuplicateKeep::First)?;
```

`DuplicateKeep` enum: `First` (keep first occurrence), `Last` (keep last), `None` (mark all duplicates).

## Random Sampling

Deterministic sampling with seed control:

```rust
// Sample n rows
let sampled = df.sample(100, None, false, Some(42))?;  // n=100, seed=42

// Sample fraction
let sampled = df.sample(0, Some(0.1), false, Some(42))?;  // 10% sample

// Sample with replacement (bootstrap)
let bootstrap = df.sample(1000, None, true, Some(42))?;

// Weighted sampling
let weighted = df.sample_weights(100, &weights_series, false, Some(42))?;
```

Uses a deterministic LCG (Linear Congruential Generator) with Fisher-Yates shuffle for reproducible results across runs. Matching seed → identical sample, regardless of platform.

## Roadmap

| Priority | Feature | Status |
|----------|---------|--------|
| High | PyO3 Python bindings | Planned; would enable `import frankenpandas as fpd` from Python |
| High | Native Datetime DType | Design phase; would replace Utf8 ISO 8601 string representation |
| Medium | Parallel execution (rayon) | Not started; architecture supports it (columns are independent) |
| Medium | DataFrame.plot() via plotters crate | Not started; would enable terminal/SVG chart output |
| Medium | Lazy evaluation / query planning | Not started; would enable optimization across chained operations |
| Low | HDF5 IO | Needs system library (libhdf5) |
| Low | Clipboard IO | Needs system clipboard access |
| Low | DataFrame.style for HTML formatting | Decorative; low priority vs correctness work |

## Key Documents

| Document | Purpose |
|----------|---------|
| `AGENTS.md` | Guidelines for AI coding agents |
| `COMPREHENSIVE_SPEC_FOR_FRANKENPANDAS_V1.md` | Full V1 specification |
| `FEATURE_PARITY.md` | Detailed parity matrix with status per feature family |
| `artifacts/perf/` | Optimization round baselines, opportunity matrices, proofs |
| `artifacts/phase2c/` | Conformance packet artifacts and drift history |

## About Contributions

Please don't take this the wrong way, but I do not accept outside contributions for any of my projects. I simply don't have the mental bandwidth to review anything, and it's my name on the thing, so I'm responsible for any problems it causes; thus, the risk-reward is highly asymmetric from my perspective. I'd also have to worry about other "stakeholders," which seems unwise for tools I mostly make for myself for free. Feel free to submit issues, and even PRs if you want to illustrate a proposed fix, but know I won't merge them directly. Instead, I'll have Claude or Codex review submissions via `gh` and independently decide whether and how to address them. Bug reports in particular are welcome. Sorry if this offends, but I want to avoid wasted time and hurt feelings. I understand this isn't in sync with the prevailing open-source ethos that seeks community contributions, but it's the only way I can move at this velocity and keep my sanity.

## License

MIT License (with OpenAI/Anthropic Rider). See `LICENSE`.
