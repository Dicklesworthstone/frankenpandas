# FrankenPandas

<div align="center">
  <img src="frankenpandas_illustration.webp" alt="FrankenPandas - Clean-room Rust reimplementation of pandas" width="600">

  **Clean-room Rust reimplementation of the full pandas API surface.**

  Drop-in pandas API in safe Rust. Python bindings (`import frankenpandas`) planned via PyO3; see the Roadmap. Zero `unsafe`. Profile-proven performance. Differential conformance against a live pandas oracle on every PR.

  ![Rust](https://img.shields.io/badge/Rust-2024_edition-orange)
  ![License](https://img.shields.io/badge/license-MIT-blue)
  ![Tests](https://img.shields.io/badge/tests-5000%2B-brightgreen)
  ![Conformance](https://img.shields.io/badge/conformance_packets-1252-blueviolet)
  ![IO Formats](https://img.shields.io/badge/IO_formats-14%2B-purple)
  ![Lines of Rust](https://img.shields.io/badge/Rust_LOC-270K-orange)
</div>

---

## TL;DR

**The Problem:** pandas is the lingua franca of data analysis, but it's single-threaded Python with unpredictable memory spikes, GIL contention in production pipelines, and dtype coercion surprises that silently corrupt results. Drop-in performance replacements (Polars, DuckDB) require rewriting your code in a different API.

**The Solution:** FrankenPandas rebuilds the entire pandas API from first principles in Rust. Same semantics, same method names, same edge-case behavior, but with columnar storage, vectorized kernels, arena-backed execution, an explicit alignment-planning phase (AACE), and compile-time safety guarantees. Every commit is verified against the actual pandas oracle.

**Why FrankenPandas?**

| Feature | pandas | Polars | FrankenPandas |
|---------|--------|--------|---------------|
| API compatibility with pandas | ✓ | Partial (different API) | **Full parity target** |
| Memory safety | Runtime errors | Safe Rust | **Safe Rust** (`#![forbid(unsafe_code)]` workspace-wide) |
| Index alignment semantics | ✓ | ✗ (no index) | **✓** (AACE: Alignment-Aware Columnar Execution) |
| Row + column MultiIndex | ✓ | ✗ | **✓** |
| Typed `DatetimeIndex` / `TimedeltaIndex` / `PeriodIndex` / `CategoricalIndex` / `RangeIndex` | ✓ | ✗ | **✓** |
| Categorical dtype | ✓ | ✓ | **✓** (metadata layer + parity accessor) |
| `eval()` / `query()` string expressions | ✓ | ✗ | **✓** (with `@local` variables, backtick columns) |
| `GroupBy.agg_named()` | ✓ | ✓ (different syntax) | **✓** |
| `merge_asof` with `tolerance` / `by` / `allow_exact_matches` | ✓ | Partial | **✓** |
| Window operations (rolling / expanding / ewm / resample) | ✓ | Partial | **✓** |
| 14+ IO formats (CSV/TSV/FWF/JSON/L/Parquet/Excel/Feather/IPC/SQL/HTML/XML/LaTeX/Markdown/Pickle/Stata/ORC/HDF5) | ✓ | Partial | **✓** (SQL is generic `SqlConnection` trait with default `rusqlite` backend; PostgreSQL/MySQL slices in progress) |
| Differential conformance against live pandas | ✗ | ✗ | **✓** (1,252 packets, 1,265 fixtures, live oracle in CI) |
| Bayesian runtime policy + evidence ledger | ✗ | ✗ | **✓** |

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

// Export to any of 14+ formats
let json = write_json_string(&summary, JsonOrient::Records)?;
let feather = write_feather_bytes(&summary)?;
let html = write_html_string(&summary)?;
let md = summary.to_markdown(true, None)?;

println!("{}", summary);
// city    avg_age  count
// NYC       32.5      2
```

## Alignment-Aware Columnar Execution (AACE)

Every binary operation between DataFrames or Series goes through an explicit index-alignment planning phase before any data is touched. An `EvidenceLedger` records each materialization decision with Bayesian confidence scores, producing an auditable execution trace.

AACE is a core identity constraint, not a best-effort optimization. pandas' alignment semantics (outer join on index for arithmetic, left join for assignment) are preserved exactly, with formal correctness evidence. Index and axis names propagate through every helper method; a recently completed fork-wide sweep retrofitted 70+ callsites in Rolling, Expanding, Ewm, Resample, SeriesGroupBy, Series, DataFrame, StringMethods, and DatetimeAccessor.

## Design Philosophy

| Principle | What It Means |
|-----------|---------------|
| **Semantic parity over speed** | Never sacrifice pandas-observable behavior for performance. Every dtype promotion, NaN propagation, index-name preservation, and output ordering contract matches pandas exactly. |
| **Prove, then optimize** | Each optimization round produces a baseline, an opportunity matrix, an isomorphism proof, and a recommendation contract. No optimization lands without correctness evidence. |
| **Fail closed** | Unknown features, incompatible dtypes, and ambiguous coercions produce errors, not silent corruption. Strict mode rejects; hardened mode logs and recovers under a Bayesian expected-loss decision rule. |
| **Zero unsafe** | Every crate uses `#![forbid(unsafe_code)]`. Memory safety comes from the type system, not audits. |
| **Test everything differentially** | Conformance packets run FrankenPandas operations and compare against the pandas oracle. 1,252 packet JSON files + 1,265 fixtures + live pandas oracle in CI on every commit. |
| **Document every divergence** | 15 known semantic divergences from pandas are written up in `crates/fp-conformance/DISCREPANCIES.md` with root-cause analysis, resolution status (ACCEPTED / INVESTIGATING / WILL-FIX), and reproducible test packets. No silent disagreement. |

## What's In The Box

The 2026-05-16 capability surface (~269k LOC of Rust across 12 crates):

| Category | Coverage |
|----------|----------|
| **DataFrame** | 960+ methods. Selection (`loc`/`iloc`/`at`/`iat`/`xs`/`squeeze`), reshaping (`melt`/`pivot_table`/`stack`/`unstack`/`crosstab`/`get_dummies`/`explode`), aggregation (`describe`/`info`/`agg`/`agg_named`/`pipe`/`apply`/`applymap`/`transform`/`combine`/`assign_fn`), statistical (`corr`/`cov`/`corrwith`/`rank`/`nlargest`/`nsmallest`), null-handling (`isna`/`dropna`/`fillna`/`ffill`/`bfill`/`interpolate`/`combine_first`/`update`), io-extension (`to_csv`/`to_json`/`to_excel`/`to_feather`/`to_parquet`/`to_html`/`to_xml`/`to_latex`/`to_markdown`/`to_pickle`/`to_stata`/`to_orc`/`to_xarray`/`to_clipboard`/`to_gbq`), display (`Display`/`to_html`/`to_latex`/`to_markdown`/`style()`), arithmetic (`add`/`sub`/`mul`/`div`/`pow`/`mod`/`floordiv` × scalar/series/df + fill_value variants), window (`rolling`/`expanding`/`ewm`/`resample` for DataFrame and GroupBy), time-series (`at_time`/`between_time`/`asof`/`shift`/`diff`/`pct_change`/`first`/`last`), MultiIndex (`set_index_multi`/`row_multiindex`/`column_multiindex`), constructors (`from_dict`/`from_records`/`from_tuples`/`from_csv`/`from_series`/`from_dict_index` + 8 more variants). |
| **Series** | 800+ methods spanning the same surfaces plus string accessor (`.str()`, 50+ methods), datetime accessor (`.dt()`, 25+ methods), timedelta accessor (`.dt().components()` etc., 15+ methods), sparse accessor (`.sparse()`), categorical accessor (`.cat()`), list accessor (`.list()`), struct accessor (`.r#struct()`, raw-identifier name), and full reduction family (sum/mean/min/max/median/std/var/sem/skew/kurt/prod/quantile/argmin/argmax/idxmin/idxmax). |
| **Index family** | Untyped `Index` + 5 typed variants: `DatetimeIndex`, `TimedeltaIndex`, `PeriodIndex`, `RangeIndex`, `CategoricalIndex`. Each typed variant carries 50+ pandas-parity methods (time-of-day accessors, set ops, slice ops, get_loc/get_indexer family, tz_localize/tz_convert, searchsorted, where/putmask, asof/asof_locs, freq/inferred_freq, mean/median/std/var/sum). `MultiIndex` is integrated with DataFrame `set_index_multi` / `xs` / `.loc[(a, b)]` / `groupby` / `reshape` / IO round-trips. |
| **GroupBy** | DataFrame-level (`DataFrameGroupBy`) and Series-level (`SeriesGroupBy`). 3 execution paths (dense Int64, arena-backed Bumpalo, HashMap fallback) with property tests proving bitwise equivalence. 14 string-dispatch aggregations + `cumsum`/`cumprod`/`cummax`/`cummin`/`rank`/`shift`/`diff`/`nth`/`head`/`tail`/`pct_change`/`value_counts`/`describe`/`get_group`/`cumcount`/`ngroup`/`pipe`/`ohlc`/`transform`/`filter`/`apply`. Window ops (`rolling`/`expanding`/`ewm`/`resample`) on both levels. |
| **Join engine** | Inner / Left / Right / Outer / Cross / Asof (Backward / Forward / Nearest). `merge_with_options` takes `MergeExecutionOptions { indicator_name, validate_mode, suffixes, sort }` with `MergeValidateMode::{OneToOne, OneToMany, ManyToOne, ManyToMany}`. `merge_asof_with_options` takes `MergeAsofOptions { allow_exact_matches, tolerance, by }`. |
| **Expression engine** | `df.eval(expr)` and `df.query(expr)`. Modulo, FloorDiv, Pow with correct precedence (`**` > unary > `*`/`/`/`//`/`%`). Bitwise shorthand (`&`/`\|`/`~`). Chained-comparison pairwise AND. `@local` variable bindings. Backtick column names. |
| **IO** | 14+ formats: CSV (with full pandas option matrix incl. `usecols`/`nrows`/`skiprows`/`dtype`/`parse_dates`/`comment`/`on_bad_lines`/`decimal`/`thousands`/`true_values`/`false_values`/`skipfooter`/`lineterminator`/`index_label`/`quote`/`escape`), TSV (`read_table`), Fixed-width (`read_fwf` with colspec inference), JSON (5 orients + Table Schema), JSONL (blank-line tolerant, key-union detection, row-cap protection), Parquet (Arrow RecordBatch), Excel (`.xlsx`/`.xls`/`.xlsb`/`.ods` with full option parity), Feather, Arrow IPC stream, SQL (generic `SqlConnection` trait + `SqlInspector` for SQLAlchemy-shaped introspection), HTML (read + write), XML (read + write + `to_xml` alias), LaTeX (file + string), Markdown (`tablefmt` accepts `"github"` / `"pipe"` / `"grid"` / `"plain"` / `"simple"`), Pickle (round-trip), Stata (round-trip), ORC (round-trip), HDF5 (snapshot, optional feature-gated backend). Deferred surfaces: `to_clipboard`, `to_gbq`, SAS reader. |
| **Type system** | `Scalar`, `DType`, `NullKind` (Null / NaN / NaT). `Timestamp`, `Timedelta`, `Period`, `Interval`, `PeriodFreq`, `IntervalClosed` as proper value types. `SparseDType` scaffolded. Coercion via `common_dtype()` / `cast_scalar()` matches pandas' Null < Bool < Int64 < Float64 hierarchy. Identity-cast fast path (AG-03) skips clone when source dtype already matches target. |
| **Runtime** | Bayesian `RuntimePolicy` (Strict / Hardened). `EvidenceLedger` with full decision trace per materialization. `ConformalGuard` for distribution-shift detection. `RaptorQEnvelope` for repair-symbol-protected durable state (conformance fixtures, benchmark baselines, migration manifests). |
| **Conformance** | 1,252 packet JSON files, 1,265+ fixture JSONs, 15 documented divergences in `DISCREPANCIES.md` (3 fully RESOLVED; remainder are ACCEPTED / INVESTIGATING / WILL-FIX with root-cause analysis), live pandas oracle in CI. Conformance tests pass (1,586 tests, 0 failures) excluding documented structural divergences. |

## Architecture

```
                  ┌─────────────────────────┐
                  │      frankenpandas      │  ← Unified facade crate
                  │   (prelude re-exports)  │
                  └────────────┬────────────┘
                               │
       ┌───────────────────────┼────────────────────────┐
       ▼                       ▼                        ▼
  ┌──────────┐      ┌──────────────────┐      ┌────────────────┐
  │ fp-expr  │      │    fp-frame      │      │     fp-io      │
  │ eval()   │      │ DataFrame        │      │  14+ formats   │
  │ query()  │      │ Series           │      │  CSV/TSV/FWF/  │
  └────┬─────┘      │ Categorical      │      │  JSON/JSONL/   │
       │            │ Rolling/Expanding│      │  Parquet/Excel/│
       ▼            │ Ewm/Resample     │      │  Feather/IPC/  │
  ┌──────────┐      │ String/Datetime  │      │  SQL/HTML/XML/ │
  │fp-runtime│      │ Sparse/List/Str. │      │  LaTeX/Markdown│
  │ Policy   │      └────────┬─────────┘      │  /Pickle/Stata/│
  │ Ledger   │               │                │  ORC/HDF5      │
  └──────────┘    ┌──────────┼──────────┐     └────────────────┘
                  ▼          ▼          ▼
              fp-index  fp-groupby  fp-join
              Alignment Arena-      Inner/Left/
              planning  backed     Right/Outer/
              5 typed   agg +      Cross/Asof
              variants  3 paths    with tolerance
                  │           │           │
                  └───────────┼───────────┘
                              ▼
                  ┌────────────┐      ┌───────────────┐
                  │fp-columnar │      │   fp-types    │
                  │ Column     │      │ Scalar/DType  │
                  │ ValidMask  │      │ Timestamp     │
                  │ Vectorized │      │ Timedelta     │
                  │ kernels    │      │ Period        │
                  └────────────┘      │ Interval      │
                                      │ NanOps (23)   │
                                      └───────────────┘

Plus two auxiliary crates: fp-conformance (1,252 packets / 1,265 fixtures)
and fp-frankentui (terminal UI dashboard, experimental).
```

The diagram above shows the runtime/data-flow crates. The total workspace is **12 crates, 269,398 lines of Rust under `crates/`** (about 250,300 of those lines live under `src/` directories, including the inline `#[cfg(test)]` modules; the remaining ~19,000 lines are out-of-`src/` test, fixture, and bench scaffolding).

## Workspace Structure

```
frankenpandas/
├── crates/
│   ├── frankenpandas/    # Unified facade crate with prelude (1,195 lines)
│   ├── fp-types/         # Scalar, DType, NullKind, Timestamp, Timedelta, Period, Interval, NanOps (4,326 lines)
│   ├── fp-columnar/      # Column, ValidityMask, vectorized kernels, full Column-API parity (8,714 lines)
│   ├── fp-index/         # Index, MultiIndex, 5 typed variants, alignment planning (20,353 lines)
│   ├── fp-frame/         # DataFrame, Series, Categorical, accessors, windows, resample (87,602 lines)
│   ├── fp-expr/          # Expression parser, eval()/query(), @local + backtick (3,337 lines)
│   ├── fp-groupby/       # GroupBy with 3 execution paths (3,314 lines)
│   ├── fp-join/          # Inner/Left/Right/Outer/Cross/Asof joins, merge_asof tolerance/by (5,349 lines)
│   ├── fp-io/            # 14+ IO formats, SqlConnection trait, SqlInspector (25,010 lines)
│   ├── fp-conformance/   # 1,252 packet JSON files, live pandas oracle, drift ledger
│   ├── fp-runtime/       # Strict/Hardened policy, EvidenceLedger, ConformalGuard, RaptorQ (2,629 lines)
│   └── fp-frankentui/    # Terminal UI dashboard (experimental, 2,755 lines)
├── artifacts/perf/       # Optimization round baselines and proofs
├── artifacts/phase2c/    # Conformance packet artifacts and drift history
└── .beads/               # Local-first issue tracker
```

## IO Format Support

| Format | Read | Write | In-Memory | File | Options |
|--------|------|-------|-----------|------|---------|
| **CSV** | `read_csv_str` / `read_csv` (path) / `read_csv_with_options_path` (path + options) | `write_csv_string` / `write_csv` (path) / `write_csv_string_with_options` | ✓ | ✓ | `CsvReadOptions` (`delimiter`, `has_headers`, `na_values`, `keep_default_na`, `na_filter`, `index_col`, `usecols`, `nrows`, `skiprows`, `dtype`, `parse_dates`, `parse_date_combinations`, `parse_date_combinations_named`, `comment`, `true_values`, `false_values`, `decimal`, `on_bad_lines`, `thousands`, `skipfooter`, `quotechar`, `escapechar`, `doublequote`, `lineterminator`); `CsvWriteOptions` (`delimiter`, `na_rep`, `header`, `include_index`, `index_label`) |
| **TSV (read_table)** | `read_table_str` / `read_table` (path) | — | ✓ | ✓ | CSV options with tab default |
| **Fixed-width** | `read_fwf_str` / `read_fwf` (path) | — | ✓ | ✓ | Explicit `colspecs` or automatic inference |
| **JSON** | `read_json_str` | `write_json_string` / `to_json` | ✓ | ✓ | 5 orients (Records / Columns / Index / Split / Values) + `Table` Schema with full Type/Format round-trip |
| **JSONL** | `read_jsonl_str` | `write_jsonl_string` | ✓ | ✓ | One object per line, blank-line tolerant, union-key detection, row-cap protection against unbounded allocation |
| **Parquet** | `read_parquet_bytes` | `write_parquet_bytes` | ✓ | ✓ | Arrow RecordBatch integration, multi-batch reading, Date32/Date64/Timestamp/Time32/Time64 conversion |
| **Excel** | `read_excel_bytes` / `read_excel_sheets` / `read_excel_sheets_ordered` (the `_ordered` variant preserves workbook sheet order) | `write_excel_bytes` / `to_excel` | ✓ | ✓ | `ExcelReadOptions` (`sheet_name`, `has_headers`, `usecols`, `names`, `index_col`, `skip_rows`); `ExcelWriteOptions` (`sheet_name`, `index`, `index_label`, `header`); supported file extensions: `.xlsx` / `.xls` / `.xlsb` / `.ods` |
| **Feather** | `read_feather_bytes` | `write_feather_bytes` | ✓ | ✓ | Arrow IPC file format (random-access footer) |
| **Arrow IPC stream** | `read_ipc_stream_bytes` | `write_ipc_stream_bytes` | ✓ | ✓ | Streaming wire format (forward-only; pipes + zero-copy interchange) |
| **SQL** | `read_sql` / `read_sql_table` / `read_sql_chunks` / `read_sql_chunks_with_options` | `write_sql` / `write_sql_with_options` | N/A | Any `SqlConnection` impl (sqlite default) | `SqlReadOptions` (`params`, `parse_dates`, `coerce_float`, `dtype`, `schema`, `columns`, `index_col`); chunking is a separate argument to `read_sql_chunks*`. `SqlWriteOptions` (`if_exists`, `index`, `index_label`, `schema`, `dtype`, `method`, `chunksize`). `SqlInspector` (SQLAlchemy-shaped: `tables`, `views`, `schemas`, `columns`, `indexes`, `foreign_keys`, `unique_constraints`, `reflect_table`, `reflect_all_tables`, `reflect_all_views`, `table_comment`, `server_version`, `max_identifier_length`) |
| **HTML** | `read_html_str` | `write_html_string` / `write_html_string_with_options` / `to_html` | ✓ | ✓ | `HtmlWriteOptions` is single-field today (`include_index: bool`); pandas option parity (`classes`, `escape`, `na_rep`, `render_links`, `table_id`, `border`, `justify`, etc.) is on the roadmap |
| **XML** | `read_xml_str` | `write_xml_string` / `to_xml` | ✓ | ✓ | `XmlWriteOptions` (`include_index`, `root_name` default `"data"`, `row_name` default `"row"`, `index_label`) |
| **LaTeX** | — | `write_latex_string` / `to_latex` / `write_latex` (path) | ✓ | ✓ | `LatexWriteOptions` (`include_index`, `na_rep`, `index_label`, `escape`); pandas' richer option matrix (caption, label, position, longtable, multicolumn/multirow) is on the roadmap |
| **Markdown** | — | `to_markdown` / `write_markdown_string_with_options` / `write_markdown` (path) | ✓ | ✓ | `MarkdownWriteOptions` (`include_index`, `na_rep`, `index_label`); `to_markdown` `tablefmt` accepts `"github"` / `"pipe"` / `"grid"` / `"plain"` / `"simple"`; default is `"github"` |
| **Pickle** | `read_pickle_bytes` | `write_pickle_bytes` | ✓ | ✓ | Round-trip via serde + bincode |
| **Stata** | `read_stata_bytes` | `write_stata_bytes` | ✓ | ✓ | Round-trip (subset of `.dta` features) |
| **ORC** | `read_orc_bytes` | `write_orc_bytes` | ✓ | ✓ | Round-trip via Arrow |
| **HDF5** | `read_hdf_*` | `to_hdf` | ✓ | ✓ (optional `hdf5` feature) | Keyed-snapshot layout (PyTables-compatible table/storer pending) |

CSV, JSON, JSONL, Parquet, Excel, Feather, SQL, HTML, XML, LaTeX, Markdown, Pickle, Stata, and ORC are accessible through `DataFrameIoExt` trait methods on `DataFrame` (e.g. `df.to_excel(path)?`, `df.to_feather(path)?`, `df.to_parquet(path)?`, `df.to_sql(&conn, "table", &opts)?`, `df.to_html_string()?`, `df.to_markdown(true, None)?`). The Arrow IPC stream format is reachable through the standalone `read_ipc_stream_bytes` / `write_ipc_stream_bytes` functions. Top-level `read_*` free functions are also re-exported through the `frankenpandas` facade.

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
| `sql-sqlite` | **on** | `fp-io/sql-sqlite` | rusqlite-backed `SqlConnection` impl + `rusqlite::Connection` re-export via the facade |
| `sql-postgresql` | off | `fp-io/sql-postgresql` | Placeholder for Phase 2 PostgreSQL adapter (no concrete bindings yet) |
| `sql-mysql` | off | `fp-io/sql-mysql` | Placeholder for Phase 2 MySQL adapter |
| `hdf5` | off | `fp-io/hdf5` | Pulls in the hdf5-metno backend for `read_hdf` / `to_hdf` |
| `tracing` | off | `fp-frame/tracing` | Emits `tracing` spans on hot paths (groupby, rolling, resample, IO) |
| `asupersync` | off | `fp-runtime/asupersync` | Pulls in the optional `asupersync` runtime integration submodule |

```toml
# Default — includes sql-sqlite
[dependencies]
frankenpandas = { path = "crates/frankenpandas" }

# No SQL deps — drop rusqlite + the sql-sqlite SqlConnection impl
frankenpandas = { path = "crates/frankenpandas", default-features = false }

# SQL + tracing spans + HDF5
frankenpandas = { path = "crates/frankenpandas", features = ["tracing", "hdf5"] }
```

When `sql-sqlite` is disabled, `read_sql` / `write_sql` and the `SqlConnection` trait remain available; just implement the trait for your own connection type to route the same APIs through it.

### Build from Source

```bash
git clone https://github.com/Dicklesworthstone/frankenpandas.git
cd frankenpandas
cargo build --workspace --release
cargo test --workspace
```

Requires **Rust nightly** (2024 edition). The exact dated nightly is pinned in `rust-toolchain.toml`; CI reads the same file-backed channel.

## Quick Start

```rust
use frankenpandas::prelude::*;

// 1. Create a DataFrame
let df = read_csv_str("ticker,price,volume\nAAPL,185.50,1000\nGOOG,140.25,500\nAAPL,186.00,1200")?;

// 2. Filter rows
let expensive = df.query("price > 150")?;

// 3. GroupBy and aggregate
let by_ticker = expensive.groupby(&["ticker"])?.sum()?;

// 4. Parse dates
let dates = Series::from_values("d", vec![0i64.into()], vec!["2024-01-15".into()])?;
let parsed = to_datetime(&dates)?;

// 5. Export to multiple formats
let csv = write_csv_string(&by_ticker)?;
let json = write_json_string(&by_ticker, JsonOrient::Records)?;
let feather = write_feather_bytes(&by_ticker)?;
let html = write_html_string(&by_ticker)?;
let md = by_ticker.to_markdown(true, None)?;

// 6. SQL round-trip — rusqlite is re-exported under the `sql-sqlite` feature
let conn = frankenpandas::rusqlite::Connection::open_in_memory()?;
write_sql(&by_ticker, &conn, "results", SqlIfExists::Fail)?;
let back = read_sql_table(&conn, "results")?;

// 7. Inspect a SQL schema like SQLAlchemy (method names drop the `list_` prefix)
let inspector = SqlInspector::new(&conn);
let tables = inspector.tables(None)?;
let cols   = inspector.columns("results", None)?;
let refl   = inspector.reflect_table("results", None)?;
```

## How It Works

### Data Model

FrankenPandas uses a columnar storage model identical to pandas' internal representation:

```
DataFrame
├── index: Index (Vec<IndexLabel>)            ← Row labels (Int64/Utf8/Timedelta64/Datetime64)
├── row_multiindex: Option<MultiIndex>        ← Optional hierarchical row index (set by set_index_multi)
├── columns: BTreeMap<String, Column>         ← Named columns
├── column_order: Vec<String>                 ← Insertion-order preservation
├── column_multiindex: Option<MultiIndex>     ← Optional hierarchical column header
└── allows_duplicate_labels: Option<bool>     ← Persisted "flags.allows_duplicate_labels"

Column
├── dtype: DType  ← {Null, Bool, Int64, Float64, Utf8, Categorical, Timedelta64, Sparse}
├── values: Vec<Scalar>                       ← Typed values
└── validity: ValidityMask                    ← Bitpacked null bitmap

Scalar  = Null(NullKind) | Bool(bool) | Int64(i64) | Float64(f64) | Utf8(String)
        | Timedelta64(i64) | Datetime64(i64) | Period(i64) | Interval(Interval)
NullKind = Null | NaN | NaT                   ← Three-way null semantics matching pandas

IndexLabel = Int64(i64) | Utf8(String) | Timedelta64(i64) | Datetime64(i64)
```

Every `Scalar` knows its own type, and every `Column` enforces type homogeneity through `DType`. The type promotion hierarchy follows pandas exactly: `Null < Bool < Int64 < Float64` (with `Utf8` incompatible with numerics). `Categorical`, `Timedelta64`, and `Sparse` are extension dtypes that ride on top of the core numeric and string buckets. `Datetime64` columns are stored as `Int64` nanosecond codes plus dtype metadata, and `Period` / `Interval` value types live in `fp-types` for scalar operations (with `PeriodIndex` / `IntervalIndex` providing the index-level surface) rather than as first-class column dtypes. The `IndexLabel` enum has a dedicated `Datetime64` variant so DatetimeIndex labels round-trip cleanly through alignment.

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
- Boolean algebra: `and_mask`, `or_mask`, `not_mask` operate word-by-word on `u64`s, processing 64 null checks per CPU instruction
- `count_valid()` uses `word.count_ones()` (hardware `POPCNT`) across all words

This is the same approach used by Apache Arrow and gives O(n/64) performance for null-aware operations instead of O(n).

### Index Alignment: The AACE Pipeline

When you write `series_a + series_b` in pandas, it silently performs an outer join on the indexes before adding. FrankenPandas makes this explicit:

```
Step 1: Plan
   align_union(left.index, right.index) → AlignmentPlan {
       union_index: Index,                    // Merged label set
       left_positions: Vec<Option<usize>>,    // Where each left value goes
       right_positions: Vec<Option<usize>>,   // Where each right value goes
   }

Step 2: Materialize
   Reindex both columns according to the plan.
   None positions → fill with NaN (missing propagation).

Step 3: Execute
   Vectorized element-wise operation on aligned arrays.

Step 4: Log
   EvidenceLedger records the decision: timestamp, mode, action,
   Bayesian posterior, expected losses.

Step 5: Propagate names
   Shared axis name (or no name when sources disagree) propagates to the result,
   matching pandas behavior across every helper method.
```

Single-label `Index::position()` lookups use **adaptive search** (AG-13): each `Index` lazily detects its own sort order in a `std::sync::OnceLock<SortOrder>`, then dispatches to O(log n) binary search for sorted Int64/Utf8 labels and an O(n) linear scan for unsorted labels. The alignment planner itself (`align_union`) builds borrowed-key HashMaps (AG-02) for the union construction regardless of sort order; the adaptive switch is on the lookup side, not the planner side. When either side has duplicate labels, the planner automatically routes through `align_non_unique` to preserve pandas' cross-product semantics.

For multi-way alignment (e.g., `DataFrame::from_series([s1, s2, s3])`), a leapfrog triejoin variant (AG-05) computes the N-way union in a single O(n log n) pass instead of iterative pairwise merges.

### Typed Index Variants

FrankenPandas exposes the same typed index family as pandas. `MultiIndexOrIndex` is the algebraic enum used internally to thread either a flat `Index` or a `MultiIndex` through the same APIs. Each typed variant carries 50+ pandas-parity methods:

| Variant | Special methods (in addition to base Index API) |
|---------|--------------------------------------------------|
| `DatetimeIndex` | `tz_localize`/`tz_convert`/`tz`, `floor`/`ceil`/`round`, `normalize`, `strftime`, `to_period`, `time`/`date`/`hour`/`minute`/`second`/`microsecond`/`nanosecond`, `year`/`month`/`day`/`dayofweek`/`dayofyear`/`quarter`/`weekofyear`/`isocalendar`, `is_month_start`/`is_month_end`/`is_quarter_start`/`is_quarter_end`/`is_year_start`/`is_year_end`/`is_leap_year`, `month_name`/`day_name`, `freq`/`inferred_freq`, `unit`/`as_unit`/`resolution`, `to_pydatetime`/`to_julian_date`, `timetz`, `asof`/`asof_locs`, `slice_locs`/`slice_indexer`/`get_slice_bound`, `searchsorted`, `where`/`putmask`, `mean`/`median`/`std`/`var`, `shift` |
| `TimedeltaIndex` | `total_seconds`, `floor`/`ceil`/`round`, `to_pytimedelta`, `components` (returns DataFrame of days/hours/minutes/seconds/milliseconds/microseconds/nanoseconds), `unit`/`as_unit`/`resolution`, `mean`/`median`/`std`/`var`/`sum`, set ops, `slice_locs`/`slice_indexer`, `searchsorted` |
| `PeriodIndex` | `to_timestamp`, `asfreq`, `freq`/`is_full`, `from_ordinals`/`from_fields`, `start_time`/`end_time`, `year`/`month`/`quarter`/`day`/`hour`/`dayofweek`/`weekofyear`, `to_flat_index`, set ops, `slice_locs`/`slice_indexer` |
| `CategoricalIndex` | `categories`/`codes`, `add_categories`/`remove_categories`/`remove_unused_categories`/`rename_categories`/`reorder_categories`/`set_categories`, `as_ordered`/`as_unordered`, `ordered` |
| `RangeIndex` | `start`/`stop`/`step`, lazy materialization, integer-arithmetic optimization for set operations |

Constructors mirror pandas: `from_tuples`, `from_arrays`, `from_product` (Cartesian product). Operations: `get_level_values(level)`, `droplevel(level)` → `MultiIndexOrIndex`, `swaplevel(i,j)`, `reorder_levels(order)`, `to_flat_index(sep)`, `from_frame(&DataFrame, &[&str])`.

### MultiIndex

Hierarchical indexing is represented as parallel label vectors (one per level):

```
MultiIndex {
    levels: Vec<Vec<IndexLabel>>,  // levels[0] = ["a","a","b","b"]
    names: Vec<Option<String>>,    //            levels[1] = [1, 2, 1, 2]
}
```

DataFrame integration carries a logical row MultiIndex alongside the flat storage index. `set_index_multi(&["col1", "col2"], drop, sep)` attaches real row-axis MultiIndex metadata; tuple-key `.loc[("a", 1)]` / `.xs(...)` / `.get_loc(...)` dispatch through that metadata; multi-key `groupby([k1, k2]).sum()` emits row-MultiIndex output; and reshape / IO round-trips preserve the row axis across `reset_index`, `stack`, `unstack`, `pivot`, `pivot_table`, CSV, Excel, Parquet, Feather, IPC, JSON, and SQL paths. `to_multi_index(&["col1", "col2"])` remains available when you want a standalone extracted MultiIndex value.

`MultiIndex` operations include `get_indexer`/`get_indexer_for`/`get_indexer_non_unique`, `slice_indexer`/`get_slice_bound`, `groupby`/`join`/`reindex`/`rename`, `searchsorted`, `all`/`any`/missing-mask, `shift`, `get_locs` (list-label parity), `truncate`, `from_frame`, `is_monotonic`/`is_lexsorted` predicates, `duplicated`/`is_unique`, `isin` (tuple- and level-aware), `insert`/`delete`/`append`/`repeat`/`dropna`.

### Period / Interval / Timestamp / Timedelta as Proper Types

FrankenPandas exposes the same scalar value types as pandas:

```rust
use frankenpandas::prelude::*;

// Timestamp: nanosecond-resolution wall clock (constructed from nanos-since-epoch)
let ts = Timestamp::from_nanos(1_705_314_600_000_000_000); // 2024-01-15T10:30:00Z (epoch seconds 1_705_314_600)
let floored = ts.floor_to_unit("H");                         // Floor to hour
let rounded = ts.round_to_unit("D");                         // Round to day
let one_day_ns: i64 = 24 * 60 * 60 * 1_000_000_000;
let plus_day = ts.add_timedelta(one_day_ns);                 // Arithmetic on nanoseconds

// Timedelta is a unit type whose associated fns operate on i64 nanoseconds.
// `Timedelta::parse` returns the nanosecond count as i64 (NAT = i64::MIN).
let td_ns: i64 = Timedelta::parse("3 days 04:15:30")?;
let secs = Timedelta::total_seconds(td_ns);                  // i64 → f64 seconds
let components = Timedelta::components(td_ns);               // TimedeltaComponents{ days, hours, minutes, ... }

// Period: a discrete calendar interval indexed by ordinal at a given freq
let q_freq = PeriodFreq::Quarterly;
let p = Period::new(/* ordinal */ 218, q_freq);             // 2024Q3 (54 yrs × 4 q + offset)

// Interval: half-open or closed bin (constructed from f64 endpoints)
let iv = Interval::new(0.0_f64, 10.0_f64, IntervalClosed::Right);
let contains = iv.contains(5.0_f64);

// Ranges: paired iterators / index builders. date_range/bdate_range/timedelta_range
// take exactly TWO of {start, end, periods}; freq is nanosecond-resolution i64.
let day_ns: i64 = 24 * 60 * 60 * 1_000_000_000;
let dates = date_range(Some("2024-01-01"), Some("2024-12-31"), None, day_ns * 30, None)?;
let bdays = bdate_range(Some("2024-01-01"), Some("2024-12-31"), None, None)?;
let tds   = timedelta_range(Some(0_i64), Some(30_i64 * day_ns), None, day_ns, None)?;
let pds   = period_range(Period::new(218, PeriodFreq::Quarterly), 12);  // start, count
let ivs   = interval_range_by_periods(0.0_f64, 1.0_f64, 10)?;

// Period <-> Timestamp on the index level (string-keyed freq + how)
let pix   = PeriodIndex::from_range(Period::new(216, PeriodFreq::Quarterly), 4);
let back  = pix.to_timestamp("S")?;     // PeriodIndex -> DatetimeIndex (start-of-period)
```

`apply_date_offset(ts, "MS")` / `infer_freq(&date_index)` provide the offset / freq inference primitives. `DataFrame::to_period(&self, freq: &str)` and `DataFrame::to_timestamp(&self, freq: &str, how: &str)` move an entire frame between Period and Datetime representations.

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
                            │ single malloc      │           │ pairs, typed         │
                            │ bulk dealloc       │           │ ScalarKey            │
                            └────────────────────┘           └──────────────────────┘
```

**Path 1, Dense Int64:** When all group keys are integers spanning ≤ 65,536 values, pre-allocates a dense array indexed directly by `key - min_key`. O(1) per-element grouping with zero hash overhead. Used for common patterns like grouping by year, month, category ID.

**Path 2, Arena-backed (Bumpalo):** When estimated intermediate memory fits within the arena budget (default 256 MB), allocates all working memory from a Bumpalo bump allocator. Single `malloc` + pointer bumps; bulk deallocation when the arena drops. Zero fragmentation, cache-friendly.

**Path 3, Global allocator HashMap with typed `ScalarKey`:** Fallback for arbitrary key types and unbounded cardinality. Stores `(source_index, accumulating_sum)` pairs and never clones the group key Scalar itself (AG-08 optimization). The original IndexLabel is reconstructed at output time from the source position. The `ScalarKey` infrastructure replaced an earlier debug-string formatting path that quietly miscompared keys across dtypes; float zeros (`-0.0` vs `0.0`) are now normalized, and dtype-aware hashing means `Int64(1)` and `Float64(1.0)` no longer collide unexpectedly.

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

When types don't align for the fast path (e.g., `Int64 + Float64`), a scalar fallback promotes each element through `cast_scalar()`, respecting the full pandas type hierarchy. For Modulo and FloorDiv, the runtime preserves Int64 when no zero divisors are present and promotes to Float64 (with `NaN`) when any zero divisor would otherwise panic, matching pandas' behavior on those edge cases. Infinity handling is pandas-compatible (`±∞ % anything → NaN`, `infinite_float divmod` matches pandas helpers).

### Expression Engine

`df.eval("a + b * c > threshold")` and `df.query("price > 100 and volume < 500")` are powered by a recursive-descent parser in `fp-expr`:

```
Grammar (simplified):
  expr       → or_expr
  or_expr    → and_expr ( ("or"|"|") and_expr )*
  and_expr   → comparison ( ("and"|"&") comparison )*
  comparison → arithmetic ( (">"|"<"|"=="|"!="|">="|"<=") arithmetic )*    // chained comparison
  arithmetic → term ( ("+"|"-") term )*
  term       → factor ( ("*"|"/"|"//"|"%") factor )*
  factor     → unary ( "**" factor )?                                       // right-assoc
  unary      → ("-"|"+"|"~"|"not") unary | atom
  atom       → NUMBER | STRING | COLUMN_NAME | `BACKTICKED COL` | @LOCAL_VAR | "(" expr ")"
```

The parser produces an `Expr` AST that the evaluator walks, resolving column references against the DataFrame's `EvalContext`. Local variables (prefixed with `@`) are broadcast to Series of the appropriate length. Column names with spaces or special characters can be referenced via backticks. Chained comparisons (`a < b < c`) parse to the pandas-style pairwise AND form (`(a < b) and (b < c)`). The entire pipeline (parse, resolve, evaluate, filter) happens in a single call with no temporary DataFrames.

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

The asymmetric loss matrix penalizes "allow if incompatible" (100.0) far more than "reject if compatible" (6.0), making the system conservative by default. Every decision is recorded in the `EvidenceLedger` with full trace: timestamp, mode, prior, posterior, Bayes factor, and the evidence terms that drove it. Recovery deadlines are enforced: a `recover_once` retry loop is bounded under buggy policy, and the codec rejects zero repair-symbol decodes.

### Categorical Data

`DType::Categorical` is a reportable dtype identity, but the physical storage is an `Int64` code column with a parallel `CategoricalMetadata` record. This dual-layer design preserves dtype reporting parity with pandas while avoiding per-element `Scalar::Categorical(...)` allocations on every match arm across the workspace:

```
Series {
    name: String,
    index: Index,
    column: Column,                  // dtype=Int64; values are integer codes (0, 1, 2, ...)
    categorical: Some(CategoricalMetadata {
        categories: Vec<Scalar>,     // ["low", "medium", "high"]
        ordered: bool,               // Whether categories have total ordering
    }),
    sparse: None,                    // Optional pandas-style sparse metadata (separate dtype layer)
}
```

The `.cat()` accessor provides pandas-compatible operations: `categories()`, `codes()`, `rename_categories()`, `add_categories()`, `remove_unused_categories()`, `set_categories()`, `as_ordered()`, `as_unordered()`, `to_values()`. Missing values use code `-1`.

### String Accessor

The `.str()` accessor provides 50+ string operations matching pandas `Series.str`:

| Category | Methods |
|----------|---------|
| Case | `lower`, `upper`, `capitalize`, `title`, `casefold` (Unicode-correct, handles German sharp s), `swapcase` (run-based for correctness) |
| Whitespace | `strip`, `lstrip`, `rstrip`, `expandtabs` (column-aware) |
| Search | `contains` (with `case`/`na`/`regex` options), `startswith` (with `na`), `endswith` (with `na`), `find`, `rfind`, `index_of`, `rindex_of` (returns **char** position, not byte) |
| Transform | `replace` (with `n`/`case`/`regex` options), `slice`, `slice_replace`, `repeat`, `pad` (with side validation), `zfill` (sign-prefix correct), `center`, `ljust`, `rjust`, `translate` (with deletion support) |
| Split/Join | `split_get`, `split_count`, `split_expand` (with `n` + padding), `rsplit_get`, `join`, `partition`, `rpartition` |
| Predicates | `isdigit` (Unicode), `isalpha` (Unicode), `isalnum`, `isspace`, `islower`, `isupper`, `isnumeric` (Unicode), `isdecimal`, `istitle` |
| Regex | `contains_regex`, `replace_regex` (with capture-group support), `replace_regex_all`, `extract` (named groups), `extractall` (named groups), `count_matches`, `findall`, `fullmatch` (case/na), `match_regex` (case/na), `split_regex_get` |
| Prefix/Suffix | `removeprefix` (Unicode-exact), `removesuffix` (Unicode-exact) |
| Wrap | `wrap` (with `drop_whitespace` option) |
| Other | `len`, `get`, `normalize` (NFC/NFD/NFKC/NFKD with combining-character support), `cat` (Series-to-Series concat with separator), `cat_series`, `get_dummies` |

### Datetime Accessor

The `.dt()` accessor provides 25+ component extraction methods. On non-datetimelike Series the accessor errors instead of silently producing NaN:

```rust
// Components
series.dt().year()?;            series.dt().month()?;
series.dt().day()?;             series.dt().hour()?;
series.dt().minute()?;          series.dt().second()?;
series.dt().microsecond()?;     series.dt().nanosecond()?;
series.dt().dayofweek()?;       series.dt().dayofyear()?;
series.dt().quarter()?;         series.dt().weekofyear()?;  // ISO week 1-53
series.dt().isocalendar()?;     // DataFrame of (year, week, day)
series.dt().date()?;            series.dt().time()?;
series.dt().timetz()?;          // time with timezone preserved

// Boundary predicates
series.dt().is_month_start()?;  series.dt().is_month_end()?;
series.dt().is_quarter_start()?; series.dt().is_quarter_end()?;
series.dt().is_year_start()?;    series.dt().is_year_end()?;
series.dt().is_leap_year()?;

// Formatting + conversion
series.dt().strftime("%Y-%m-%d %H:%M")?;
series.dt().month_name()?;       // locale is fixed (English) — pandas tz/locale flag is on the roadmap
series.dt().day_name()?;
series.dt().to_timestamp()?;     // For Period-typed series (string-keyed conversion is on DataFrame / typed Index)

// Rounding
series.dt().floor("H")?;         // Truncate to hour (string unit: "H", "D", "min", "S", "ms", "us", "ns")
series.dt().ceil("D")?;          // Round up to day
series.dt().round("min")?;       // Round to nearest minute (banker's rounding)

// Timezone (string-keyed; rejects unknown IANA names)
series.dt().tz_localize(Some("America/New_York"))?;
series.dt().tz_convert(Some("UTC"))?;
```

### Timedelta Accessor

The `.dt.components()` method returns a DataFrame with `days`/`hours`/`minutes`/`seconds`/`milliseconds`/`microseconds`/`nanoseconds` columns. Plus `total_seconds`, `td_neg`, `td_abs`, `td_ratio` for scalar operations on `Timedelta64` Series.

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

`merge_asof` now supports the full pandas option matrix: `tolerance` (reject matches further than N), `by` (equality constraint on grouping columns), and `allow_exact_matches` (whether `right_key == left_key` counts). The right-side dtype is preserved in output, suffixes are honored, and direction is validated up front.

### Window Operations

Full rolling, expanding, exponentially-weighted, and resample window support on both Series and DataFrame:

```rust
// Rolling window (like df["col"].rolling(30).mean())
let ma_30 = series.rolling(30, None).mean()?;
let vol   = series.rolling(252, Some(20)).std()?;          // window=252, min_periods=20
let centered = series.rolling_with_center(30, None, true).mean()?;

// Expanding window (cumulative)
let cum_max = series.expanding(None).max()?;
let cum_var = series.expanding(Some(3)).var()?;

// Exponentially weighted moving average
let ewma = series.ewm(Some(20.0), None).mean()?;           // span=20
let ewstd = series.ewm(None, Some(0.5)).std()?;            // alpha=0.5

// Time-based resampling
let monthly = series.resample("M").sum()?;
let bday    = series.resample("B").mean()?;
let weekly  = series.resample("W").agg("ohlc")?;
```

| Window Type | Series Methods | DataFrame Methods |
|-------------|---------------|-------------------|
| **Rolling** | `sum`, `mean`, `min`, `max`, `std`, `var`, `count`, `median`, `quantile`, `apply`, `corr`, `cov`, `rank`, `first`, `last`, `skew`, `kurt`, `agg` | Same |
| **Expanding** | `sum`, `mean`, `min`, `max`, `std`, `var`, `median`, `apply`, `corr`, `cov`, `count`, `skew`, `kurt` | Same |
| **EWM** | `mean`, `sum`, `std`, `var`, `cov`, `corr` | Same |
| **Resample** | `sum`, `mean`, `count`, `min`, `max`, `first`, `last`, `ohlc`, `median`, `std`, `var`, `agg`, `size`, `transform`, `get_group` | Same |

GroupBy also supports `rolling()`, `expanding()`, `ewm()`, and `resample()` for within-group window operations, with full pandas-parity (validate `min_periods <= window` up front; reject invalid `freq` strings eagerly; first/last guard against all-missing windows; max/prod return NaN on empty windows).

### Reshaping

All major pandas reshaping operations:

```rust
// Long → Wide
let pivoted = df.pivot_table("revenue", "region", "product", "sum")?;
let pivoted_multi = df.pivot_table_multi_values(&["revenue", "quantity"], "region", "product", "sum")?;
let pivoted_multi_agg = df.pivot_table_multi_agg("revenue", "region", "product", &["sum", "mean", "count"])?;
let pivoted_with_margins = df.pivot_table_with_margins_name("revenue", "region", "product", "sum", true, "Grand Total")?;
let pivoted_filled = df.pivot_table_fill("revenue", "region", "product", "sum", 0.0)?;

// Wide → Long
let melted = df.melt(&["id"], &["q1", "q2", "q3"], Some("quarter"), Some("sales"))?;

// Stack / Unstack (preserves source index name)
let stacked = df.stack()?;
let unstacked = stacked.unstack()?;

// Contingency tables
let gender = Series::new("gender", df.index().clone(), df.column("gender").unwrap().clone())?;
let dept   = Series::new("department", df.index().clone(), df.column("department").unwrap().clone())?;
let ct      = DataFrame::crosstab(&gender, &dept)?;
let ct_norm = DataFrame::crosstab_normalize(&gender, &dept, "all")?;  // "all" / "index" / "columns"

// One-hot encoding
let dummies = df.get_dummies(&["color", "size"])?;

// Cross-section selection
let row = df.xs(&"2024-01-15".into())?;

// Explode list-valued column to multiple rows
let exploded = df.explode("tags", ",")?;       // splits "a,b,c" → 3 rows; sep is required

// Transpose
let t = df.transpose()?;
```

### DataFrame Output Formats

The `Display` trait + `DataFrameIoExt` extension trait reach **20+ output methods**:

| Method | pandas Equivalent | Format |
|--------|-------------------|--------|
| `to_csv(sep, include_index)` | `df.to_csv()` | Comma/tab-separated values |
| `to_csv_options(opts)` | `df.to_csv(...)` | Full pandas option matrix |
| `to_json(orient)` | `df.to_json()` | JSON with 5 orients + Table Schema |
| `to_jsonl_file(path)` | `df.to_json(lines=True)` | One object per line |
| `to_string()` | `df.to_string()` | Pandas-aligned ASCII table |
| `to_string_table(include_index)` | `df.to_string(index=...)` | Aligned ASCII table with explicit index control |
| `to_string_truncated(idx, rows, cols)` | `df.to_string(max_rows=)` | Truncated with head/tail + "..." |
| `to_html(include_index)` | `df.to_html()` | HTML `<table>` |
| `to_html_string_with_options(opts)` | `df.to_html(...)` | Full pandas option matrix |
| `to_xml(opts)` | `df.to_xml()` | XML root + row element naming |
| `to_latex(include_index)` | `df.to_latex()` | LaTeX `tabular` |
| `to_markdown(include_index, tablefmt: Option<&str>)` | `df.to_markdown(tablefmt=...)` | GitHub/pipe (when `tablefmt=None`), `Some("grid")`, `Some("plain")` text output |
| `to_dict(orient)` | `df.to_dict()` | dict/list/records/index/split/tight |
| `to_series_dict()` | `df.to_dict('series')` | `BTreeMap<String, Series>` |
| `to_records()` | `df.to_records()` | Vec of row vectors |
| `to_numpy_2d()` | `df.to_numpy()` | `Vec<Vec<f64>>` |
| `to_parquet`/`to_feather`/`to_excel`/`to_sql`/`to_stata`/`to_pickle`/`to_orc`/`to_xarray` | Same | Binary / persistent formats |
| `style()` | `df.style` | `StyledDataFrame` with HTML rendering |
| `Display` trait | `print(df)` | Column-aligned with shape footer |

### GroupBy: Complete Aggregation Matrix

14 aggregation functions available through string dispatch:

| Function | Returns | Notes |
|----------|---------|-------|
| `sum` | Float64 (or Int64 / Bool preserved where pandas does) | Null-skipping; Utf8 concat supported |
| `mean` | Float64 | Null-skipping |
| `count` | Int64 | Non-null count |
| `min` | Same as input | Null-skipping; Utf8 lexicographic; Int64/Bool preserved |
| `max` | Same as input | Null-skipping; Utf8 lexicographic; Int64/Bool preserved |
| `std` | Float64 | Sample standard deviation (ddof=1); configurable |
| `var` | Float64 | Sample variance (ddof=1); configurable |
| `median` | Float64 | Middle value |
| `first` | Same as input | First non-null; Int64 stays Int64 (no Float64 promotion) |
| `last` | Same as input | Last non-null; Int64 stays Int64 |
| `prod` | Float64 (or Int64 / Bool preserved) | Product of values |
| `sem` | Float64 | Standard error of mean |
| `skew` | Float64 | Fisher's skewness (NaN on too-few values, matching pandas) |
| `kurt`/`kurtosis` | Float64 | Excess kurtosis (NaN on too-few values, matching pandas) |

Plus group-level operations: `cumsum`, `cumprod`, `cummax`, `cummin`, `rank`, `shift`, `diff`, `nth`, `head`, `tail`, `pct_change`, `value_counts`, `describe`, `get_group`, `cumcount`, `ngroup`, `pipe`, `ohlc`, `transform`, `filter`, `apply`. The `agg_named()` form takes `(output_name, column, func)` tuples and rejects duplicate output names.

### Datetime Parsing

`to_datetime()` auto-detects common formats and normalizes to ISO 8601:

| Input Format | Example | Auto-Detected? |
|--------------|---------|----------------|
| ISO 8601 date | `2024-01-15` | ✓ |
| ISO 8601 datetime | `2024-01-15T10:30:00` | ✓ |
| Space-separated | `2024-01-15 10:30:00` | ✓ |
| Slash date | `2024/01/15` | ✓ |
| US date (MM/DD/YYYY) | `01/15/2024` | ✓ |
| Epoch seconds (Int64) | `1705312200` | ✓ |
| Epoch milliseconds | `1705312200000` | ✓ (auto-detected from magnitude > 10^11) |
| Custom format | `15-Jan-2024` | Via `to_datetime_with_format(s, Some("%d-%b-%Y"))` |

`to_datetime` also supports a `unit=` parameter (`s`/`ms`/`us`/`ns`/`m`/`h`/`D`/`Y`), a `utc=` flag, and an `origin=` parameter (`unix`/`julian`/numeric/string-date). Mixed naive/tz-aware strings are coerced via `utc=True`; without `utc=True`, mixed-tz inputs fall back to raw strings (see DISC-012). Timezone support: `tz_localize(tz)` (with `ambiguous`/`nonexistent` policies), `tz_convert(tz)` using `chrono-tz` for IANA timezone names.

`to_timedelta()` parses duration strings with similar flexibility:

```rust
// All of these work:
// "02:30:45"          → HH:MM:SS
// "3 days 04:15:30"   → pandas-style timedelta
// "5 days"            → day-only
// "3 hours"           → natural language
// Int64(3661)         → seconds (→ "01:01:01")
// Negative: "-3 days 04:00:00", "-02:30:45"
```

### Describe: Statistical Summary

`DataFrame::describe()` generates the same 8-row statistical summary as pandas:

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

Supports custom percentiles (`describe_with_percentiles(&[0.1, 0.5, 0.9])`) and dtype-filtered describe (`describe_dtypes(&["number"], &[])` for numeric only, or include `"object"` for string columns which produce `count`/`unique`/`top`/`freq`).

### Correlation and Covariance

Pairwise correlation and covariance matrices:

```rust
let pearson  = df.corr()?;                  // Pearson (default)
let spearman = df.corr_method("spearman")?; // Rank correlation
let kendall  = df.corr_method("kendall")?;  // Kendall tau
let cov_mat  = df.cov()?;                   // Covariance matrix
let corr_w   = df.corrwith(&other_df)?;     // Column-wise correlation
let corr_w_axis = df.corrwith_axis(&other_df, 1)?;  // Row-wise (axis=1)

// With min_periods (drop pairs with too few overlapping non-nulls)
let corr_strict = df.corr_min_periods(20)?;
let cov_strict  = df.cov_min_periods(20)?;
```

Series-level: `series.corr(&other)`, `series.cov_with(&other)`, `series.autocorr(lag)`.

### Apply and Transform

Multiple ways to apply custom logic:

```rust
// Element-wise on each column
let transformed = df.applymap(|scalar| { /* transform */ })?;
let transformed = df.applymap_na_action(|s| { /* skips nulls */ }, true)?;

// Row-wise with full row access (returns Series; first arg is the result column name)
let result: Series = df.apply_row("row_total", |row_values: &[Scalar]| {
    row_values[0].clone()
})?;

// Shape-preserving transform via per-element closure
let doubled = df.transform(|s: &Scalar| match s {
    Scalar::Int64(v)   => Scalar::Int64(v * 2),
    Scalar::Float64(v) => Scalar::Float64(v * 2.0),
    other              => other.clone(),
})?;

// Named-aggregation broadcast on GroupBy
let group_means = df.groupby(&["region"])?.transform("mean")?;

// Column assignment with closures (pandas df.assign(new_col=lambda df: ...))
let df2 = df.assign_fn(vec![
    ("ratio", Box::new(|df| {
        let rev = df.column("revenue").expect("revenue column");
        let cost = df.column("cost").expect("cost column");
        let values: Vec<Scalar> = rev.values().iter().zip(cost.values()).map(|(r, c)| {
            match (r, c) {
                (Scalar::Float64(a), Scalar::Float64(b)) => Scalar::Float64(a / b),
                _ => Scalar::Null(NullKind::NaN),
            }
        }).collect();
        Column::from_values(values).map_err(FrameError::from)
    })),
])?;

// Pipe for method chaining
let result = df
    .pipe(|d| d.sort_values("x", true))?
    .pipe(|d| d.head(10))?;
```

### Module-Level Functions

Functions matching pandas top-level API:

| Function | pandas Equivalent | Description |
|----------|-------------------|-------------|
| `concat_dataframes` | `pd.concat` | Concatenate along axis 0 or 1 with join modes |
| `concat_dataframes_with_keys` | `pd.concat(keys=)` | Hierarchical index labeling |
| `concat_dataframes_with_axis_join` | `pd.concat(axis=, join=)` | Full pandas join mode coverage |
| `concat_dataframes_with_ignore_index` | `pd.concat(ignore_index=True)` | Reindex to 0..n |
| `concat_series` | `pd.concat` for Series | Same options |
| `to_datetime` | `pd.to_datetime` | Parse dates with unit/origin/utc/format |
| `to_timedelta` | `pd.to_timedelta` | Parse durations from strings/seconds |
| `to_numeric` | `pd.to_numeric` | Coerce to numeric with NaN for failures |
| `cut` | `pd.cut` | Equal-width binning (O(1) per element) |
| `qcut` | `pd.qcut` | Quantile-based binning (binary search) |
| `date_range` | `pd.date_range` | Date range builder (rejects overflow / over-specified ranges) |
| `bdate_range` | `pd.bdate_range` | Business-day range |
| `timedelta_range` | `pd.timedelta_range` | Timedelta range |
| `period_range` | `pd.period_range` | Period range (weekly business / fields constructor available) |
| `interval_range_by_periods` / `interval_range_by_step` | `pd.interval_range` | Interval range |
| `infer_freq` | `pd.infer_freq` | Frequency inference |
| `merge_asof` | `pd.merge_asof` | With `tolerance` / `by` / `allow_exact_matches` |

## Performance

Five named optimization rounds with formal evidence, plus an ongoing complexity sweep that converted dozens of O(n²) hot paths to O(n) or O(n + k) via HashMap/HashSet/IsinIndex.

| Round | Optimization | Speedup |
|-------|-------------|---------|
| Round 1 | Remove duplicate `run_fixture_operation` invocation inside `run_fixture` | Eliminates redundant per-fixture work |
| Round 2 | `align_union` borrowed-key HashMap (AG-02) | Eliminates index clones |
| Round 3 | GroupBy identity-alignment fast path (AG-11) | Skips reindex when indexes match |
| Round 4 | Dense Int64 aggregation path (AG-06) | O(1) array access, no HashMap |
| Round 5 | `has_duplicates` OnceLock memoization | **87% faster** on groupby benchmark |

### Recent complexity sweep (2026-05)

The 2026-05 hot-path sweep landed ~20 separate O(n²) → O(n) reductions, every one of them measured under the perf regression gate:

- `Series::value_counts` / `unique` / `nunique_with_dropna` / `duplicated` / `drop_duplicates`
- `Series::map` / `replace` / `map_with_default` / `isin` (Series + DataFrame)
- `pd.cut` (O(1)-bucket via sorted edges), `pd.qcut` (binary search)
- `Series::mode_with_dropna` / `mode_values` (cross-dtype path)
- `DataFrame::nunique` / `value_counts` / `append` / `get_dummies`
- `Series::drop` / `unstack` / `str.get_dummies` / `factorize`
- `fp-columnar::factorize_with_options`
- CSV NA HashSet, Excel sheet HashSet

### Measured Baselines (10K rows, debug profile)

| Operation | p50 | p95 | Notes |
|-----------|-----|-----|-------|
| Join (inner, 50% overlap) | 12ms | 19ms | HashMap-based equijoin |
| Join (outer, 50% overlap) | 27ms | 37ms | Union index construction |
| Join (right, 50% overlap) | 14ms | 21ms | Budget-tracked via perf gate |
| Boolean filter (5 cols) | 15ms | 20ms | Reindex all columns by mask |
| `head(100)` | 22us | 28us | O(1) slice, no copy |
| Scalar add (5 cols) | 2.5ms | 2.8ms | Per-column vectorized |
| Scalar comparison (5 cols) | 3.0ms | 4.1ms | Per-column with Bool output |

Performance baselines tracked for join (inner/left/right/outer), filter (boolean mask, head/tail), and DataFrame arithmetic at 10K–100K row scales. Benchmarks run via `cargo test -p fp-conformance --test perf_baselines -- --ignored --nocapture`, and a perf regression gate runs them in CI with budgets enforced.

## Optimization Catalog

FrankenPandas applies **13 named optimization techniques** with inline `AG-NN` markers in source (`AG-02`, `AG-03`, `AG-05` through `AG-15`), drawn from the alien-graveyard systems-pattern library. The IDs `AG-01` and `AG-04` are reserved for techniques surveyed during the optimization rounds but not shipped (or rolled into adjacent techniques). Each landed technique is independently toggled and proven via isomorphism tests; the table below highlights 10 of the 13 (the remaining `AG-09`, `AG-12`, `AG-15` are documented in their respective module headers under `crates/`):

| ID | Technique | Where Applied | What It Does |
|----|-----------|---------------|--------------|
| AG-02 | Borrowed-key HashMap | fp-index `align_union`, fp-join build phase | Builds position maps using `&IndexLabel` references instead of cloning labels. Eliminates O(n) allocations in the join build phase. |
| AG-03 | Identity-cast skip | fp-types `cast_scalar_owned` | When source dtype already equals target dtype, returns the value without cloning. Saves one allocation per element in column coercion. |
| AG-05 | N-way leapfrog triejoin | fp-index `multi_way_align` | Computes the union of N indexes in a single O(n log n) sorted-merge pass instead of iterative pairwise O(n² log n). Used by `DataFrame::from_series`. |
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

Extension dtypes (layered on top of the core hierarchy):
  Datetime64, Timedelta64, Period, Interval, Categorical, Sparse
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
| Int64 | Int64 (with zero divisor for `%`/`//`) | Float64 (with NaN) | Promote to avoid panic, match pandas |

The identity-cast optimization (AG-03) detects when source dtype already matches target dtype and skips the clone entirely. The `infer_dtype(values)` function folds `common_dtype()` across all elements to find the narrowest type that fits. This is used during CSV/JSON parsing where cell types are inferred individually and then unified per-column. Float-zero normalization (`-0.0` vs `0.0`) is applied to all key paths (`HashMap` keys, `groupby` keys, `nannunique` set membership, frame uniqueness).

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
- `semantic_eq` bridges all Null kinds (so `Null` and `NaN` compare equal in semantic sense)
- Aggregation skips nulls by default: `nansum([1, NaN, 3]) → 4`
- GroupBy with `dropna=true` excludes null group keys

The `ValidityMask` makes null checking O(1) per element (single bit test) and O(n/64) for bulk operations (word-level popcount).

## NanOps: Null-Aware Aggregation Library

The `fp-types` crate provides 19 null-skipping scalar reductions plus 4 cumulative-transform helpers (23 `pub fn nan*` functions total) that underpin every statistical operation in the codebase:

```rust
// Scalar reductions (all skip Null/NaN automatically):
nansum(&values)         // → Scalar (sum, Float64 / Int64 / Bool / Utf8 / Timedelta64)
nanmean(&values)        // → Scalar (mean, Float64 / Timedelta64)
nancount(&values)       // → Scalar::Int64 (count of non-missing)
nanmin(&values)         // → same type as input minimum (Utf8 lexicographic; Int64/Bool preserved)
nanmax(&values)         // → same type as input maximum
nanmedian(&values)      // → Scalar (Float64 / Timedelta64)
nanvar(&values, ddof)   // → Scalar::Float64 (sample variance, ddof=1 default)
nanstd(&values, ddof)   // → Scalar::Float64 (sample std dev)
nansem(&values, ddof)   // → Scalar::Float64 (standard error of mean)
nanprod(&values)        // → Scalar (Float64 / Int64 / Bool with numpy-style wrap)
nanptp(&values)         // → same type as input range (max - min; Timedelta64 supported)
nanskew(&values)        // → Scalar::Float64 (Fisher's skewness; NaN on <3 values)
nankurt(&values)        // → Scalar::Float64 (excess kurtosis; NaN on <4 values)
nanquantile(&values, q) // → Scalar (linear-interp quantile; Timedelta64 supported)
nanargmax(&values)      // → Option<usize> (position of maximum)
nanargmin(&values)      // → Option<usize> (position of minimum)
nannunique(&values)     // → Scalar::Int64 (count of unique non-missing; float-zero normalized)
nanany(&values)         // → Scalar::Bool (true if any non-missing truthy)
nanall(&values)         // → Scalar::Bool (true if every non-missing is truthy)

// Cumulative transforms (return Vec<Scalar> — propagate nulls in place):
nancumsum(&values)      // → cumulative sum (Float64 / Utf8 concat / Timedelta64)
nancumprod(&values)     // → cumulative product (Float64)
nancummax(&values)      // → running maximum (input dtype; Utf8 lexicographic; Timedelta64)
nancummin(&values)      // → running minimum (input dtype; Utf8; Timedelta64)
```

These are the building blocks for `Series::sum()`, `DataFrame::mean()`, `GroupBy::std()`, `describe()`, and every other statistical method. The "skip nulls by default" behavior matches pandas' `skipna=True` default. Empty inputs or all-null inputs return `NaN` for float aggregations and `0` for count, matching pandas exactly.

## Error Architecture

Every crate has its own typed error enum, all implementing `std::error::Error` + `Display`, all `#[non_exhaustive]` so future variant additions stop being semver-major:

| Error Type | Crate | Key Variants |
|-----------|-------|--------------|
| `TypeError` | fp-types | `IncompatibleDtypes { left, right }`, `TimedeltaParseError`, `TimedeltaOverflow`, `DateRangeOverflow` |
| `ColumnError` | fp-columnar | `LengthMismatch { left, right }`, `DTypeMismatch { left, right }`, `InvalidMaskType { dtype }`, `InvalidSorter { len, reason }` |
| `IndexError` | fp-index | `OutOfBounds { position, length }`, `LengthMismatch { expected, actual, context }`, `InvalidAlignmentVectors` |
| `FrameError` | fp-frame | `LengthMismatch { index_len, column_len }`, `CompatibilityRejected(String)`, `Column(ColumnError)`, `Index(IndexError)` (transparent wrappers via `#[from]`) |
| `ExprError` | fp-expr | `ParseError(String)`, `UnknownSeries(String)`, `UnknownLocal(String)` |
| `JoinError` | fp-join | `Frame(FrameError)`, `Column(ColumnError)` (transparent wrappers; no join-specific variants today) |
| `GroupByError` | fp-groupby | `Frame(FrameError)`, `Index(IndexError)`, `Column(ColumnError)` (transparent wrappers) |
| `IoError` | fp-io | `MissingHeaders`, `MissingIndexColumn(...)`, `Csv(...)`, `Json(...)`, `Parquet(...)`, `Excel(...)`, `Arrow(...)`, `Sql(...)`, `Html(...)`, `Xml(...)`, `Hdf5(...)`, `Stata(...)`, `Pickle(...)`, `Orc(...)` |
| `RuntimeError` | fp-runtime | `ClockSkew` (today). Most runtime error conditions surface as `DecisionAction::Reject` in the `EvidenceLedger` rather than as enum variants; recovery deadlines, decode failures, and ConformalGuard rejections are logged as decisions, not thrown as `RuntimeError`. |

All error types are re-exported through the `frankenpandas` facade crate.

## Testing

| Category | Count | What It Covers |
|----------|-------|----------------|
| fp-frame unit tests | ~1,936 | DataFrame, Series, Categorical, MultiIndex integration |
| fp-io tests | ~532 | 14+ IO formats, adversarial inputs, round-trip correctness, full SQL trait surface (introspection / chunking / per-column dtype overrides) |
| fp-columnar tests | ~296 | Column / ColumnData / ValidityMask / arithmetic / comparison / sparse encoding |
| fp-index tests | ~368 | Index alignment, MultiIndex, duplicate detection, typed Index variants |
| fp-types tests | ~176 | DType / Scalar / Timestamp / Timedelta / Period / Interval / NanOps |
| fp-join tests | ~73 | Inner/Left/Right/Outer/Cross/Asof joins, merge_asof edge cases |
| fp-groupby tests | ~66 | Per-aggregation kernels (sum / mean / std / median / nunique / ...), HyperLogLog approximate counting |
| fp-expr tests | ~64 | Expression parsing, eval/query, @local variables, chained comparison, backtick columns |
| fp-runtime tests | ~40 | RuntimePolicy, EvidenceLedger, ConformalGuard, RaptorQ envelopes, recovery deadlines |
| fp-frankentui tests | ~22 | TUI snapshot model, E2E scenario harness |
| **fp-conformance tests** | **1,252 packets + hundreds of live-oracle tests** | Differential conformance against pandas oracle (live + fixture replay) |
| Property-based (proptest) | 100+ | DType coercion, IO round-trip (CSV/JSON/SQL/Excel/Feather/Parquet/Arrow IPC), ValidityMask algebra, DataFrame arithmetic invariants, stateful op-chain |
| Fuzz harnesses | **30 targets** under `fuzz/fuzz_targets/` | Parquet IO, Arrow IPC stream, scalar cast, Series arithmetic, groupby_sum, join, column arithmetic, Excel IO, index alignment, shift metamorphic, eval/query, semantic_eq, pivot_table dispatch, groupby agg dispatch, rolling window, parallel + TSan, SQL read, DataFrame constructor + merge |
| Metamorphic tests | Several families | Null/NaN, join/reshape, value_counts, cumsum round-trip, quantile interpolation invariants, cov_with_options, GroupBy idxmin/idxmax Utf8, shift inner-overlap |

**Total: 5,173 `#[test]` markers in `src/`, plus hundreds of live-pandas-oracle test cases, plus 1,252 packet JSON files with 1,265 fixtures.**

### Conformance Gate

```bash
./scripts/phase2c_gate_check.sh
```

Regenerates conformance packet artifacts and fails closed if any parity report or gate is not green. **1,252 packet JSON files spanning 1,265 fixtures** cover alignment, join, groupby, concat, filter, CSV, dtype, null semantics, resample, rolling, groupby rolling/resample, datetime accessors, string accessors, MultiIndex, IO round-trip, and more. The live pandas oracle runs in CI on every PR (with system-pandas fallback). The drift history ledger (`artifacts/phase2c/drift_history.jsonl`) tracks parity trends over time.

**14 documented divergences** in [`crates/fp-conformance/DISCREPANCIES.md`](crates/fp-conformance/DISCREPANCIES.md): 2 in the "Resolved Divergences" section (DISC-005 and DISC-013), the remaining 12 labeled ACCEPTED / INVESTIGATING / WILL-FIX. Each carries full root-cause analysis, status, affected test cases, and a review date so users hitting these failures find the explanation without having to re-derive the divergence.

## Missing Data Handling

### Detection

```rust
let nulls = series.isna()?;        // Bool Series: true where null/NaN
let valid = series.notna()?;       // Bool Series: true where valid
let count = series.count();        // Count of non-missing values
let has   = series.hasnans();      // true if any missing values exist
let cnt_na = df.count_na()?;       // Per-column null counts
```

DataFrame-level: `df.isna()`, `df.notna()`, `df.isnull()`, `df.notnull()` return DataFrames of Booleans. `first_valid_index()` and `last_valid_index()` scan for the first/last non-null row.

### Filling

```rust
// Fill with constant
let filled = series.fillna(&Scalar::Float64(0.0))?;

// Forward / backward fill (with optional limit)
let ffilled = series.ffill(None)?;
let bfilled = series.bfill(Some(2))?;

// Linear interpolation (and other methods)
let interp = series.interpolate()?;
let cubic = series.interpolate_method("cubic")?;

// Combine two DataFrames, filling nulls from `other`
let combined = df.combine_first(&other)?;

// Update in place from another DataFrame
let updated = df.update(&other)?;

// Per-column fillna with a dict (BTreeMap<String, Scalar>)
use std::collections::BTreeMap;
let fill_map: BTreeMap<String, Scalar> = BTreeMap::from([
    ("price".to_owned(),  Scalar::Float64(0.0)),
    ("volume".to_owned(), Scalar::Int64(0)),
]);
let filled = df.fillna_dict(&fill_map)?;
```

### Dropping

```rust
// Drop rows with any null (default)
let clean = df.dropna()?;

// Drop rows where ALL values are null
let clean = df.dropna_with_options(DropNaHow::All, None)?;

// Drop rows with nulls in specific columns only
let clean = df.dropna_with_options(DropNaHow::Any, Some(&["price".into(), "volume".into()]))?;

// Drop rows with fewer than N non-null values (thresh)
let clean = df.dropna_with_threshold(3, None)?;

// Drop COLUMNS with nulls (axis=1)
let clean = df.dropna_columns()?;
```

## Type Coercion and Conversion

```rust
// Cast Series to a specific dtype
let float_col = int_series.astype(DType::Float64)?;

// Cast DataFrame column
let df2 = df.astype_column("price", DType::Float64)?;

// Cast multiple columns at once, including pandas-style string conversion
let df2 = df.astype_columns(&[("count", DType::Int64), ("score", DType::Utf8)])?;

// Cast with error policy (matches pandas `errors=` parameter)
let df2 = df.astype_safe(DType::Int64, "raise")?;     // "raise" / "ignore"

// Auto-infer best dtypes (Utf8 → Int64/Float64 where possible)
let df2 = df.convert_dtypes()?;
let df2 = df.infer_objects()?;

// Coerce to numeric with NaN for failures
let numeric = to_numeric(&string_series)?;
```

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
let diff  = df1.sub_df(&df2)?;
let ratio = df1.div_df(&df2)?;
let product = df1.mul_df(&df2)?;
let mod_df = df1.mod_df(&df2)?;
let pow_df = df1.pow_df(&df2)?;

// With fill_value for missing alignment positions
let sum = df1.add_df_fill(&df2, 0.0)?;
let radd_df = df1.radd(&df2)?;               // pandas radd — DataFrame ‘r’ versions are generic over the operand type
```

### Cumulative and Sequential

```rust
let csum  = df.cumsum()?;
let cprod = df.cumprod()?;
let cmax  = df.cummax()?;
let cmin  = df.cummin()?;
let delta = df.diff(1)?;
let moved = df.shift(1)?;
let pct   = df.pct_change(1)?;

// Axis-1 variants
let csum_x = df.cumsum_axis1()?;
let diff_x = df.diff_axis1(1)?;
let shift_x = df.shift_axis1(1)?;
let pct_x  = df.pct_change_axis1(1)?;
```

### Clipping and Rounding

```rust
let clipped = df.clip(Some(0.0), Some(100.0))?;
let lower   = df.clip_lower(0.0)?;
let upper   = df.clip_upper(100.0)?;
let rounded = df.round(2)?;
let absolute = df.abs()?;

// Series-level: Utf8 supported via lexicographic clip
let clipped_str = string_series.clip_with_series(&lo_series, &hi_series)?;
```

### Replacement

```rust
let cleaned = df.replace(&[
    (Scalar::Int64(-999), Scalar::Null(NullKind::NaN)),
])?;

let mapped = series.map_with_na_action(&mapping, true)?;  // na_action=ignore

// Conditional assignment (case_when)
let n = scores.len();
let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
let value_a = Series::from_values("grade", labels.clone(), vec!["A".into(); n])?;
let value_b = Series::from_values("grade", labels,         vec!["B".into(); n])?;
let graded = scores.case_when(&[
    (scores.ge_scalar(&Scalar::Int64(90))?, value_a),
    (scores.ge_scalar(&Scalar::Int64(80))?, value_b),
])?;
```

## Advanced Selection Methods

```rust
// Top-N and Bottom-N rows by column value (Utf8 supported)
let top5 = df.nlargest(5, "revenue")?;
let bot3 = df.nsmallest(3, "price")?;
let top5_all = df.nlargest_keep(5, "revenue", "all")?;       // 'first' / 'last' / 'all'
let top5_multi = df.nlargest_multi(5, &["revenue", "volume"])?;

// Find label of min/max value
let worst_day = series.idxmin()?;
let best_day  = series.idxmax()?;

// Value counts
let counts = series.value_counts()?;
let pcts = series.value_counts_with_options(true, true, false, true)?;  // normalize, sort, ascending, dropna
let binned = series.value_counts_bins(10)?;
let subset = df.value_counts_subset(&["region", "year"])?;

// Membership testing
let mask = series.isin(&[Scalar::Utf8("A".into()), Scalar::Utf8("B".into())])?;
let in_range = series.between(&Scalar::Int64(10), &Scalar::Int64(20), "both")?;  // "both" / "neither" / "left" / "right"
let dict_match = df.isin_dict(&map)?;

// Index-based position lookup
let pos = series.searchsorted(&Scalar::Float64(42.0), "left")?;
let (codes, uniques) = series.factorize()?;  // O(n + k) now

// Select columns by dtype
let numeric_only = df.select_dtypes(&[DType::Int64, DType::Float64], &[])?;
let non_numeric = df.select_dtypes(&[], &[DType::Int64, DType::Float64])?;

// Flexible label-based filtering
let subset = df.filter_labels(Some(&["price", "volume"]), None, None, 1)?;
let regex_match = df.filter_labels(None, None, Some("^rev"), 1)?;
let row_filter = df.filter_axis(None, None, Some("^row_a"), 0)?;  // (items, like, regex, axis)

// Reindex
let reindexed = series.reindex(new_labels)?;
let with_method = series.reindex_with_method(new_labels, "ffill")?;  // "ffill" / "bfill" / "nearest"
let with_fill = df.reindex_fill(new_labels, &Scalar::Float64(0.0))?;
let trimmed = series.truncate(Some(&start_label), Some(&end_label))?;

// Column manipulation
let (popped_series, remaining_df) = df.pop("temp_col")?;
let with_new = df.insert(2, "computed", new_column)?;        // Rejects out-of-bounds loc
let renamed = df.add_prefix("raw_")?;
let renamed = df.add_suffix("_v2")?;
let dropped = df.drop_columns(&["x", "y"])?;
let dropped = df.drop_rows_int(&[0, 2, 5])?;
```

## DataFrame Introspection

```rust
let shape = df.shape();              // (nrows, ncols)
let dtypes = df.dtypes()?;           // Series indexed by column name; values are DType strings
let info = df.info();                // String summary (like df.info() in pandas)
let mem = df.memory_usage()?;        // Per-column byte estimates as Series
let ndim = df.ndim();                // Always 2 for DataFrame
let axes = df.axes();                // (index, column_names)
let is_empty = df.is_empty();        // True if zero rows
let allows_dups = df.allows_duplicate_labels();  // Persisted duplicate-label flag (Option<bool>)

// Deep equality (structural + value comparison)
let same = df1.equals(&df2);

// Element-wise diff between two DataFrames
let changes = df1.compare(&df2)?;             // pandas DataFrame.compare semantics

// Squeeze single-column/single-row DataFrames. Returns Result<Series, Self>;
// pattern match instead of using `?` (the Err arm isn't an `Error` impl).
let series  = single_col_df.squeeze(1).map_err(|_| FrameError::CompatibilityRejected("not single-axis".into()))?;
let one_row = single_cell_df.squeeze(0).map_err(|_| FrameError::CompatibilityRejected("not single-axis".into()))?;

// Scalar access
let val = series.iat(0)?;
let val = series.at(&label)?;

// Lookup specific (row, col) pairs
let values = df.lookup(&row_labels, &col_names)?;

```

## SeriesGroupBy

Series-level groupby provides a lightweight API for single-column aggregation:

```rust
let by_region = revenue_series.groupby(&region_series)?;

let sums  = by_region.sum()?;
let means = by_region.mean()?;
let stds  = by_region.std()?;
let meds  = by_region.median()?;
let prods = by_region.prod()?;
let unique = by_region.nunique()?;
let running = by_region.cumsum()?;
let first_rows = by_region.head(2)?;
let group_a = by_region.get_group("A")?;
let sampled = by_region.sample(Some(5), None, false, Some(42))?;
let q90 = by_region.quantile(0.90)?;

// Multiple aggregations at once
let multi = by_region.agg(&["sum", "mean", "count", "nunique"])?;
```

`SeriesGroupBy` supports: `sum`, `mean`, `count`, `min`, `max`, `std`, `var`, `median`, `first`, `last`, `prod`, `size`, `any`, `all`, `nunique`, `idxmin`, `idxmax`, `rank`, `rank_with_pct`, `cumcount`, `ngroup`, `cumsum`, `cumprod`, `cummin`, `cummax`, `shift`, `diff`, `pct_change`, `head`, `tail`, `nth`, `get_group`, `keys`, `groups`, `indices`, `ngroups`, `ndim`, `dtype`, `is_monotonic_increasing`, `is_monotonic_decreasing`, `sample`, `take`, `quantile`, `sem`, `skew`, `value_counts`, `unique`, `ohlc`, `transform`, `filter`, `apply`, `pipe`, `describe`, `rolling`, `expanding`, `ewm`, `resample`, and the multi-function `agg`.

## Sorting

```rust
// Sort by column values
let sorted = df.sort_values("price", true)?;          // ascending=true

// Control NaN position
let sorted = series.sort_values_na(true, "first")?;   // NaN at top (validated)
let sorted = series.sort_values_na(true, "last")?;    // NaN at bottom (default)

// Sort by index labels
let sorted = df.sort_index(true)?;
```

## Concat: Full Options

```rust
let stacked = concat_dataframes(&[&df1, &df2])?;
let wide    = concat_dataframes_with_axis(&[&df1, &df2], 1)?;
let inner   = concat_dataframes_with_axis_join(&[&df1, &df2], 1, ConcatJoin::Inner)?;
let labeled = concat_dataframes_with_keys(&[&df1, &df2], &["train", "test"])?;
let clean   = concat_dataframes_with_ignore_index(&[&df1, &df2])?;
```

## Pivot Tables: Full Options

```rust
let pt = df.pivot_table("revenue", "region", "product", "sum")?;
let pt = df.pivot_table_multi_values(&["revenue", "quantity"], "region", "product", "sum")?;
let pt = df.pivot_table_with_margins("revenue", "region", "product", "sum", true)?;
let pt = df.pivot_table_with_margins_name("revenue", "region", "product", "sum", true, "Grand Total")?;
let pt = df.pivot_table_fill("revenue", "region", "product", "sum", 0.0)?;
let pt = df.pivot_table_multi_agg("revenue", "region", "product", &["sum", "mean", "count"])?;
```

## Time-Series Operations

```rust
// Select rows at a specific time (DatetimeIndex required)
let noon = df.at_time("12:00:00")?;
let morning = df.between_time("09:00:00", "12:00:00")?;

// As-of: backward search by label (second arg is Option<&[&str]> subset of columns)
let snap = df.asof(&"2024-01-15".into(), None)?;

// Period↔Timestamp conversion on DataFrame (string-keyed for freq + how)
let dft = df_period.to_timestamp("Q", "S")?;   // freq="Q", how="S" (start) or "E" (end)
let dfp = df_dt.to_period("M")?;                // freq="M" (monthly)
```

## Column Manipulation

```rust
// Rename columns
let renamed = df.rename_with(|name| format!("col_{name}"))?;
let prefixed = df.add_prefix("input_")?;
let suffixed = df.add_suffix("_raw")?;
let r = df.rename_index_with(|i| format!("idx_{i}"))?;          // Preserves axis name
let r = df.rename_index(&map)?;                                  // Mapping rename

// Assign new column (value vector)
let df2 = df.assign_column("computed", computed_values)?;

// Reorder columns
let reordered = df.select_columns(&["id", "name", "value"])?;    // Preserves row_multiindex
```

## Recipes

### Financial Data Pipeline

```rust
use frankenpandas::prelude::*;
use std::path::Path;

let trades = read_csv_str(
    "date,ticker,price,volume\n\
     2024-01-15,AAPL,185.50,1000\n\
     2024-01-15,GOOG,140.25,500\n\
     2024-01-16,AAPL,186.00,1200\n\
     2024-01-16,GOOG,141.00,800"
)?;

let date_series = Series::new("date", trades.index().clone(),
    trades.column("date").unwrap().clone())?;
let parsed_dates = to_datetime(&date_series)?;

let vwap = trades.groupby(&["ticker"])?.agg_named(&[
    ("total_value", "price", "sum"),
    ("total_vol",   "volume", "sum"),
    ("trade_count", "volume", "count"),
])?;

write_jsonl(&vwap, Path::new("daily_vwap.jsonl"))?;
```

### Merge-Asof with Tolerance

```rust
use frankenpandas::prelude::*;

// Join trades with quotes at the nearest preceding timestamp, but only if
// the quote is no more than 5 minutes stale (300 seconds, expressed as f64
// in the unit of the join key — here: epoch seconds).
let merged = merge_asof_with_options(
    &trades, &quotes, "ts_seconds", AsofDirection::Backward,
    MergeAsofOptions {
        tolerance: Some(300.0),
        by: Some(vec!["ticker".to_owned()]),
        allow_exact_matches: true,
    },
)?;
let result = DataFrame::new(merged.index, merged.columns)?;
```

### MultiIndex Operations

```rust
let mi = MultiIndex::from_product(vec![
    vec!["east".into(), "west".into()],
    vec![2023i64.into(), 2024i64.into()],
])?
.set_names(vec![Some("region".into()), Some("year".into())]);

let regions = mi.get_level_values(0)?;
let flat = mi.to_flat_index("_");                // "east_2023", "east_2024", ...
// from_frame takes Vec<(level_name, Vec<IndexLabel>)> — build it manually from columns
let levels = vec![
    (Some("region".to_owned()), df.column("region").unwrap().values().iter().map(|s| IndexLabel::from(s.clone())).collect()),
    (Some("year".to_owned()),   df.column("year").unwrap().values().iter().map(|s| IndexLabel::from(s.clone())).collect()),
];
let mi2 = MultiIndex::from_frame(levels)?;

// Set on DataFrame and round-trip through IO
let df2 = df.set_index_multi(&["region", "year"], true, "_")?;
let row = df2.xs(&"east".into())?;                // Cross-section by first level

// Tuple-key loc: the row_multiindex API takes a slice of IndexLabels for the levels.
// (IndexLabel has no tuple variant — multi-level lookup goes through `xs` chains
// or the MultiIndex.get_loc / get_locs APIs.)
let cell = df2.xs(&"east".into())?.xs(&2024i64.into())?;

// Save through Parquet — MultiIndex round-trips
write_parquet_bytes(&df2)?;
```

### Period Range and Resample

```rust
use frankenpandas::prelude::*;

// Build a quarterly PeriodIndex via the typed builder (ordinal-based)
let pix = PeriodIndex::from_range(
    Period::new(216, PeriodFreq::Quarterly),   // start ordinal
    12,                                        // 12 periods → 2024Q1..2026Q4
);

let revenue_values: Vec<Scalar> = vec![
    100.0.into(), 120.0.into(), 130.0.into(), 150.0.into(),
    110.0.into(), 140.0.into(), 160.0.into(), 180.0.into(),
    115.0.into(), 145.0.into(), 170.0.into(), 200.0.into(),
];
let revenue = Series::new("revenue", pix.to_index(), Column::from_values(revenue_values)?)?;

// Resample to yearly with mean (DatetimeIndex required; convert Period -> Timestamp first)
let dt_index = pix.to_timestamp("S")?;  // "S" = start-of-period; returns DatetimeIndex
let dt_series = Series::new("revenue", dt_index.into_index(), revenue.column().clone())?;
let yearly = dt_series.resample("Y").mean()?;
```

### Expression-Driven Analysis

```rust
use std::collections::BTreeMap;

// Compute new columns with eval
let profit = df.eval("revenue - cost")?;

// Filter with compound conditions
let hot_deals = df.query("price < 50 and rating > 4.0")?;

// Backtick-quoted column with a space
let with_space = df.query("`monthly revenue` > 10000")?;

// Chained comparison (parses to pairwise AND)
let normal = df.query("0 < temperature < 100")?;

// Local variables
let locals = BTreeMap::from([("threshold".to_owned(), Scalar::Float64(100.0))]);
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

**Packet families** cover: series alignment (FP-P2C-001–003), join semantics (FP-P2C-004), groupby aggregates (FP-P2C-005, 011), concat (FP-P2C-006), null/NaN ops (FP-P2C-007), CSV round-trip (FP-P2C-008), dtype invariants (FP-P2C-009), filter/selection (FP-P2C-010), plus **400+ DataFrame-level packets** (FP-P2D-014 through FP-P2D-433+) covering merge, concat axis options, head/tail, loc/iloc, sort, constructor variants, transpose, top-N, insert, assign, rename, reindex, drop, replace, where, mask, shift axis=1, describe, corr, cov, idxmin/idxmax, sem, skew, kurtosis, prod, sum, mean, std, var, min, max, median, any, all, nunique, quantile, value_counts, memory_usage, and IO round-trips for every format.

The **live oracle** runs in CI on every PR. Locally, when `/dp/frankenpandas/legacy_pandas_code/pandas` exists, the harness invokes pandas via a Python subprocess with stdin worker-thread drain to avoid pipe deadlock; when it doesn't, the harness falls back to fixture replay. The CI pipeline always exercises both modes via a pinned `oracle/requirements.txt`. Every parity report gets a **RaptorQ repair-symbol sidecar** for bit-rot detection, and the drift history ledger (`artifacts/phase2c/drift_history.jsonl`) tracks parity trends over time.

## Selection and Indexing

```rust
// Label-based (like df.loc[])
let row = df.loc(&["row_label".into()])?;
let subset = df.loc(&["a".into(), "b".into()])?;

// Position-based (like df.iloc[])
let row = df.iloc(&[0])?;
let last = df.iloc(&[-1])?;             // Negative indexing resolves from end
let slice = df.head(10)?;
let tail = df.tail(5)?;                 // Supports negative n

// Boolean masking
let mask = df.query("price > 100")?;
let filtered = df.filter_rows(&bool_series)?;

// Conditional replacement
let filled = df.where_mask_df(&cond_df, &Scalar::Float64(0.0))?;
let filled_with_df = df.where_cond_df(&cond_df, &other_df)?;
let masked = df.mask_df(&cond_df, &Scalar::Null(NullKind::NaN))?;
let masked_with_df = df.mask_df_other(&cond_df, &other_df)?;

// Index operations
let reindexed = df.set_index("date", true)?;          // Column → index
let with_multi = df.set_index_multi(&["a", "b"], true, "_")?;  // Multi-column → row MultiIndex
let reset = df.reset_index(false)?;
let sorted = df.sort_index(true)?;
let deduped = df.drop_duplicates(None, DuplicateKeep::First, false)?;
```

## Serialization and Interoperability

All core types are serializable via serde:

```rust
// Every type derives Serialize + Deserialize:
// Scalar, DType, NullKind, IndexLabel, Index, MultiIndex, typed Index variants,
// Series, DataFrame, CategoricalMetadata, Column, ValidityMask, Timestamp,
// Timedelta, Period, Interval, SparseDType

let json = serde_json::to_string(&scalar)?;       // Tagged enum: {"kind":"int64","value":42}
let back: Scalar = serde_json::from_str(&json)?;

let bytes = bincode::serialize(&dataframe)?;
let back: DataFrame = bincode::deserialize(&bytes)?;
```

The `Scalar` enum uses serde's tagged representation for human-readable JSON. `column_order` is preserved on serde round-trip. Legacy `"str"`/`"string"` aliases are accepted for the `Utf8` `DType`.

**Arrow interop:** DataFrame ↔ Arrow RecordBatch conversion is built in (used by Parquet, Feather, and IPC stream IO). Series ↔ Arrow Array conversion is exposed via `series_from_arrow_array` / `series_to_arrow_array`. This means FrankenPandas data can be zero-copy shared with any Arrow-compatible system (DuckDB, DataFusion, Spark via Arrow Flight).

## Adversarial and Property-Based Testing

Beyond unit tests, FrankenPandas employs:

### Property-Based Tests (proptest)

**100+ properties** that must hold for ALL randomly-generated inputs:

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
| JSON round-trip | Shape preserved across Records/Columns/Split/Values/Table | 100 |
| SQL round-trip | Shape and values preserved through SQLite | 50 |
| Excel round-trip | Int64 values recovered from f64 through xlsx | 50 |
| Feather / Parquet / IPC round-trip | Shape, names, and values exact | 100 |
| DataFrame arithmetic | `add_scalar` preserves shape, `add(0)` ≡ identity | 100 |
| Comparison ops | `eq` XOR `ne` = true for non-NaN values | 100 |
| Cumsum round-trip | Cumulative reductions are monotonic | 50 |
| Quantile interpolation | Q(0) == min, Q(1) == max, Q(0.5) ≈ median | 50 |
| Stateful op-chain | Sequences of mutations are commutative where claimed | 50 |

### Fuzz Harnesses (30 targets)

Parquet IO, Arrow IPC stream, scalar cast, Series::add, groupby_sum, join_series, column arithmetic, feather IO, common_dtype, excel IO, index alignment, shift metamorphic, abs/clip/round/between/cumulative-extrema/nlargest/sort_values/rank/diff/cum sum-prod/idxmax-idxmin/where-mask/fillna/dropna/replace/isin/duplicated/sort_index/reindex/take/set_axis/rename/truncate/drop/head-tail/isna-notna/count/sum/mean/std-var/median, cross-format round-trip, dataframe eval, `semantic_eq`, `pivot_table` dispatch, groupby agg dispatch, rolling window min_periods, stateful op-chain, parallel + TSan, SQL read, DataFrame constructor + merge.

Hardening: libFuzzer memory/timeout bounds, structure-aware dictionaries + sanitizer instrumentation, weekly corpus minimization (cron), regression corpus tested on every PR.

### Adversarial Input Tests (16+ packets)

- **CSV:** 200K-character field, 1000 columns, embedded newlines in quotes, multi-byte UTF-8 (Japanese, Cyrillic, emoji), header-only files, no trailing newline, empty rows, malformed `na_values` set
- **JSON:** Deeply nested objects, `i64::MAX`/`i64::MIN` boundary values, near-`f64::MAX` floats, empty `Records` arrays, empty `Columns` objects, bare `NaN` tokens
- **SQL:** 10,000-row batch insert, column names with spaces (quoted identifier handling), column names with embedded double-quotes, SQL injection rejection (Bobby Tables pattern), identifier-length validation, schema-qualified DROP rejection
- **XML:** Deeply nested elements, attribute injection, mixed content
- **Excel:** Header-only sheets, sheet-order preservation, headerless reads

## Duplicate Handling

```rust
let mask = df.duplicated(None, DuplicateKeep::First)?;
let unique = df.drop_duplicates(None, DuplicateKeep::First, false)?;
let deduped = series.drop_duplicates()?;

// Index-level
let has_dups = index.has_duplicates();             // O(1) after first call (OnceLock)
let unique_idx = index.drop_duplicates_keep(DuplicateKeep::First);

// Persisted flag (pandas DataFrame.flags.allows_duplicate_labels)
let df = df.set_flags(Some(false))?;       // pass Some(true)/Some(false)/None
```

`DuplicateKeep` enum: `First` (keep first occurrence), `Last` (keep last), `None` (mark all duplicates).

## Random Sampling

```rust
// Sample n rows (n is Option<usize>; use None when supplying frac)
let sampled = df.sample(Some(100), None, false, Some(42))?;        // n=100, seed=42, validates frac when supplied
let sampled = df.sample(None, Some(0.1), false, Some(42))?;        // 10% sample
let bootstrap = df.sample(Some(1000), None, true, Some(42))?;      // With replacement

// Weighted sampling (validates non-negative, non-NaN weights)
let weights: Vec<f64> = (0..df.len()).map(|i| (i + 1) as f64).collect();
let weighted = df.sample_weights(100, &weights, false, Some(42))?;
```

Uses a deterministic LCG (Linear Congruential Generator) with Fisher-Yates shuffle for reproducible results across runs. Matching seed → identical sample, regardless of platform.

## Troubleshooting

| Problem | Cause | Fix |
|---------|-------|-----|
| `cargo build` fails with "edition 2024" error | Requires Rust nightly | Install via `rustup toolchain install nightly` and check `rust-toolchain.toml` |
| `Column::from_values` returns `IncompatibleDtypes` | Mixed Utf8 + numeric values in one column | Ensure homogeneous types, or use `to_numeric()` first |
| `query("col > 5")` returns error | Column name not found in DataFrame | Verify column exists with `df.column_names()`; quote with backticks if name has spaces |
| CSV round-trip changes Float64 to Int64 | `1.0` written as `"1"`, parsed back as Int64 | Use `dtype` parameter in `CsvReadOptions` to force types |
| SQL write fails with "table already exists" | Default `SqlIfExists::Fail` policy | Use `SqlIfExists::Replace` or `SqlIfExists::Append` |
| SQL read returns `UnsupportedSchema` | Pre-2.0 backend doesn't expose schema introspection | Implement `SqlConnection::list_schemas` (returns empty vec is fine) |
| Excel round-trip loses integer precision for large values | Excel stores all numbers as f64 | Values within `i64` range with zero fraction are recovered as Int64 |
| JSONL reader rejects file with "row cap exceeded" | Anti-DoS row cap to prevent unbounded allocation | Streaming chunks: split the input upstream, or pre-validate row count |
| `to_datetime` returns NaT | Unrecognized date format | Use `to_datetime_with_format(series, Some("%d/%m/%Y"))` with explicit format |
| Mixed naive/tz-aware datetimes parse as strings | Without `utc=True`, pandas falls back to raw strings (DISC-012) | Pass `utc=True` to force normalization |
| `str.find` position differs from byte offset | `find` returns **char** position to match pandas | Use `find_byte` if you need byte offsets (note: not exposed in pandas; usually you want char) |
| `merge_asof` returns surprising matches | Direction defaults to backward | Specify `direction=AsofDirection::{Backward,Forward,Nearest}` explicitly |
| `MultiIndex.get_locs` raises on partial keys | List labels require all-level matching by default | Use `MultiIndex::get_loc_level(label, level)` for single-level lookup |
| `DataFrame.transpose` errors with duplicate index | pandas raises when transposing with duplicate row labels | Reset_index first, or drop duplicates |

## Limitations

| Limitation | Status | Workaround |
|-----------|--------|------------|
| No Python bindings yet | PyO3 bindings planned (`br-frankenpandas-4clx` release umbrella) | Use the Rust API directly, or interop via Feather/Parquet for hand-off |
| SQL has one bundled backend (`rusqlite`) | The generic `SqlConnection` trait + `SqlInspector` is feature-complete; PostgreSQL/MySQL/MS-SQL/Oracle slices are tracked under `br-frankenpandas-fd90` | Use SQLite via `rusqlite::Connection::open[_in_memory]`, or implement `SqlConnection` for another backend |
| Single-threaded execution | No parallel execution yet | Profile-proven fast paths + arena-backed groupby compensate; rayon parallelism on the roadmap |
| Native plot rendering deferred | `DataFrame::plot` / `hist` / `boxplot`, `Series::plot` / `hist`, and GroupBy plotting hooks now return backend-neutral `PlotSpec` / `HistogramSpec` / `BoxPlotSpec` data while the plotters/charming renderer is pending | Feed the returned specs to an external renderer, or use Feather/Parquet/CSV export with pandas/matplotlib |
| Clipboard IO is deferred | System clipboard dependency | Use CSV/JSON string export and copy through the host application |
| GBQ IO is deferred | Google Cloud SDK dependency | Export to Parquet/CSV and use `bq load` |
| SAS reader is deferred | Read-only proprietary format | Convert externally with `sas7bdat` or `pyreadstat` first |
| Native Datetime DType is internally `Int64` ns timestamps | Datetime/Timedelta/Period scalars exist but DataFrame columns store nanosecond Int64 codes | Use the `.dt()` accessor for component extraction; for serde, use `to_period` / `to_timestamp` to normalize |
| Sparse storage is dense under the hood | `SparseDType` is reportable and the `SparseAccessor` API works, but `Column` storage is still `Vec<Scalar>` (see DISC-009) | Use the `.sparse()` accessor to interrogate density / nnz on a Series; compressed-sparse physical storage is a future epic |
| GroupBy.apply has shape-explicit variants | Rust static typing forces `apply_scalar` / `apply_series` / `apply_series_stacked` (see DISC-010) instead of pandas' shape-inferring `apply` | Pick the variant that matches your closure's output shape |
| Int64 → Float64 promotion on null introduction | We do not yet emulate pandas' nullable-extension Int64 type (DISC-011 / DISC-014) | Aggregations that introduce nulls produce Float64; cast back to Int64 with `astype` if downstream code requires it |
| Mixed naive/tz-aware CSV `parse_dates` falls back to raw strings | Without `utc=True`, normalization is ambiguous (DISC-012) | Pass `utc=True` to `to_datetime` or `CsvReadOptions` |

## FAQ

**Q: How compatible is this with pandas?**
A: We target absolute API parity. The same method names, same parameter names, same edge-case behavior. Differential conformance tests verify against the actual pandas oracle on every PR. **1,252 packet JSON files + 1,265 fixtures + live oracle in CI** is the current evidence. 14 divergences are documented in `DISCREPANCIES.md`, every one with root-cause analysis, affected tests, and a reproducible packet.

**Q: Why not just use Polars?**
A: Polars is excellent but has a different API (lazy evaluation, no index alignment, different method names). FrankenPandas targets Rust teams that want pandas semantics rather than Polars' query-planner style: identical method names, identical edge-case behavior, identical alignment rules. Python drop-in requires the planned PyO3 bindings; today FrankenPandas is a Rust library.

**Q: Is `unsafe` code used anywhere?**
A: No. Every crate in the workspace uses `#![forbid(unsafe_code)]`. Memory safety comes from the Rust type system, not audits.

**Q: How do I use this from Python?**
A: PyO3 bindings are a planned future step. Currently this is a pure Rust library.

**Q: What's the `EvidenceLedger`?**
A: Every alignment decision, dtype coercion, and policy override is logged with Bayesian confidence scores and the evidence terms (log-likelihood ratios) that drove each decision. This creates an auditable trail of exactly how your data was transformed. pandas makes these same decisions silently with no record.

**Q: What's the performance like?**
A: Five formal optimization rounds with measured evidence, plus an ongoing complexity sweep that converted ~20 O(n²) hot paths to O(n) or O(n+k) in 2026-05. All optimizations include isomorphism proofs showing behavior is unchanged. The perf regression gate runs measured baselines in CI with budgets enforced.

**Q: How is the GroupBy implemented?**
A: Three automatic execution paths. Dense Int64 keys (range ≤ 65,536) use O(1) array indexing. Medium cardinality uses Bumpalo arena allocation (single malloc, zero fragmentation). Everything else falls back to a HashMap keyed by a typed `ScalarKey` with source-index referencing to avoid per-group clones. All three produce identical results, verified by property-based tests.

**Q: What does "clean-room" mean?**
A: We never read, reference, or copy from the pandas source code. We study pandas' *behavior* (input → output contracts, edge cases, dtype rules) via the conformance oracle, then implement from first principles in Rust. This avoids any license contamination and often produces better implementations.

**Q: How do you handle NaN vs None vs NaT?**
A: Three distinct null kinds: `NullKind::Null` (generic missing), `NullKind::NaN` (float not-a-number), `NullKind::NaT` (not-a-time). `Float64(NaN)` also counts as missing. `is_missing()` returns true for all of these. `semantic_eq()` treats `NaN == NaN` as true (unlike IEEE 754), matching pandas behavior, and bridges all Null kinds.

**Q: What's the memory overhead vs pandas?**
A: Higher per-cell than pandas' numpy-backed primitives. `Column` persistently stores `Vec<Scalar>` (a tagged enum per element); each cell carries the dtype tag plus the largest-variant footprint, so a Float64 column is roughly 3-4× the bytes of an equivalent numpy `float64` array. The trade-off buys uniform null-handling and dtype-erased generic kernels. Where it matters, AG-10 materializes a contiguous typed view (`ColumnData::{Float64(Vec<f64>), Int64(Vec<i64>), ...}`) on the hot path so vectorized arithmetic still runs against native slices and SIMD auto-vectorizes. `ValidityMask` uses 1 bit per element (vs pandas' 8-byte nullable dtype), and arena-backed GroupBy/Join operations avoid per-group heap fragmentation.

**Q: How do typed Index variants differ from a base `Index`?**
A: A base `Index` holds heterogeneous `IndexLabel`s (Int64 / Utf8 / Datetime64 / Timedelta64 / Period). The typed variants (`DatetimeIndex`, `TimedeltaIndex`, `PeriodIndex`, `RangeIndex`, `CategoricalIndex`) enforce homogeneity and expose pandas-parity methods specific to that type (e.g., `DatetimeIndex::tz_localize`, `PeriodIndex::asfreq`, `RangeIndex` lazy materialization). They live inside the `MultiIndexOrIndex` algebraic type that flows through the public API.

**Q: Can I use this for production ETL pipelines?**
A: The core DataFrame, IO, and GroupBy/Join operations are solid and well-tested (**5,000+ in-source tests** including adversarial inputs, property-based fuzzing, and a live pandas oracle in CI). This is pre-1.0 software, so API stability is not yet guaranteed, but the correctness bar is high. The known divergences are documented in `DISCREPANCIES.md`.

**Q: Why are there so few open beads?**
A: At the time of this writing, **only 2 of 1,988 tracked beads remain open** (`br-frankenpandas-ctmet` for dtype drift in 25 conformance packets, and `br-frankenpandas-qrn2w` for `groupby_sum` dropping Timedelta64). The project went through a 19-pass review-mode audit in 2026-04 that triaged the entire surface, and follow-up sessions through 2026-05 closed all but those two.

## Roadmap

| Priority | Feature | Status |
|----------|---------|--------|
| High | 0.1.0 release to crates.io with signed tag | Tracked by `br-frankenpandas-4clx`; release-plz workflow already in CI |
| High | PyO3 Python bindings | Planned; would enable `import frankenpandas as fpd` from Python |
| High | PostgreSQL `SqlConnection` adapter | Tracked by `br-frankenpandas-fd90` slices 2-3; `sql-postgresql` placeholder feature already in place |
| High | MySQL `SqlConnection` adapter | `br-frankenpandas-fd90` slice 3; `sql-mysql` placeholder feature already in place |
| Medium | Native nullable Int64 (DISC-011 / DISC-014 fix) | Required to close 25 dtype-drift packets in `br-frankenpandas-ctmet` |
| Medium | Parallel execution (rayon) | Architecture supports it (columns are independent) |
| Medium | Native plotting via plotters/charming | Public plotting hooks present and return `PlotSpec` / `BoxPlotSpec` data; backend implementation would enable PNG/SVG output |
| Medium | Lazy evaluation / query planning | Would enable optimization across chained operations |
| Low | Native HDF5 PyTables-compatible table/storer layouts | `read_hdf` / `to_hdf` provide a keyed snapshot surface today (feature-gated) |
| Low | Clipboard IO | Needs system clipboard access |
| Low | `to_gbq` Google BigQuery writer | Needs Google Cloud SDK |
| Low | SAS reader | Read-only proprietary format |

## Key Documents

| Document | Purpose |
|----------|---------|
| `AGENTS.md` | Guidelines for AI coding agents |
| `CHANGELOG.md` | Three-phase change history; Phase 1 capability foundation, Phase 2 parity completion (sub-phases 2a/2b/2c) |
| `COMPREHENSIVE_SPEC_FOR_FRANKENPANDAS_V1.md` | Full V1 specification |
| `FEATURE_PARITY.md` | Detailed parity matrix with status per feature family |
| `COVERAGE_MATRIX.md` | Test/conformance coverage matrix |
| `CATEGORICAL_COVERAGE.md` | Categorical accessor coverage report |
| `PANIC_CONTRACT_COVERAGE.md` | Per-API `# Panics` contract enforcement |
| `ERROR_CONFORMANCE.md` | Pandas error catalog and FrankenPandas error parity status |
| `DIFFERENTIAL_FUZZ_DESIGN.md` | Differential fuzz design notes |
| `crates/fp-conformance/DISCREPANCIES.md` | 14 documented divergences from pandas (DISC-005 + DISC-013 fully RESOLVED; 12 ACCEPTED / INVESTIGATING / WILL-FIX) |
| `artifacts/perf/` | Optimization round baselines, opportunity matrices, proofs |
| `artifacts/phase2c/` | Conformance packet artifacts, drift history, compat-closure attestation packs |

## Glossary of Project-Specific Terms

| Term | Definition |
|------|-----------|
| **AACE** | *Alignment-Aware Columnar Execution*. The crown-jewel architectural pattern: every binary op routes through an explicit `AlignmentPlan` build phase before any data is materialized. Each plan is recorded in the `EvidenceLedger` with a Bayesian posterior, expected losses, and the evidence terms that drove the decision. |
| **Alignment Plan** | A `(union_index, left_positions, right_positions)` triple emitted by `align_union` / `align_inner` / `align_left`. `*_positions[i] = Some(j)` means "the value at output position `i` comes from input position `j`"; `None` means "missing, fill with NaN/Null". The plan is validated in debug mode (AG-14). |
| **ValidityMask** | A bitpacked null bitmap: one bit per element, packed into `Vec<u64>`. `1` = valid, `0` = null. Used by Apache Arrow too. Enables 64-elements-per-instruction null algebra via word-level AND / OR / NOT and `POPCNT`. |
| **ScalarKey** | A typed, dtype-aware hash key replacing the older debug-string formatting path. Normalizes `-0.0`/`0.0`, gives `Int64(1)` and `Float64(1.0)` separate identities, and is the canonical groupby/HashMap key across `fp-frame` and `fp-groupby`. |
| **Bumpalo arena** | A bump allocator from the `bumpalo` crate. GroupBy and Join may route intermediate allocations into a per-operation arena with a configurable budget (default 256 MB). One `malloc`, pointer-bump on every push, single bulk dealloc on drop. Zero fragmentation, cache-friendly. |
| **Adaptive sort-order lookup (AG-13)** | An `Index` lazily detects whether its labels are sorted ascending. The check runs once, the result is cached in a `std::sync::OnceLock<SortOrder>`, and `Index::position()` then dispatches to O(log n) binary search for sorted Int64/Utf8 or O(n) linear scan otherwise. |
| **Leapfrog triejoin (AG-05)** | A worst-case-optimal N-way join algorithm adapted to N-way index alignment. Single O(n log n) sorted-merge pass replaces iterative pairwise O(n² log n) for `DataFrame::from_series` and other N-way unions. |
| **EvidenceLedger** | An append-only structured log of every policy-driven decision the runtime makes: alignment choices, dtype coercions, repair vs reject, etc. Each entry carries timestamp, mode (Strict/Hardened), action, prior, posterior, Bayes factor, and the contributing evidence terms with their log-likelihood ratios. |
| **ConformalGuard** | A statistical wrapper that produces calibrated prediction sets via split-conformal inference. Used by `fp-runtime` to flag operations whose inputs fall outside the calibration distribution, giving distribution-shift detection without distributional assumptions. |
| **RaptorQ envelope** | A repair-symbol-protected wrapper around durable artifacts (conformance fixture bundles, benchmark baselines, migration manifests, reproducibility ledgers, long-lived state snapshots). Each envelope carries a manifest, an integrity scrub report, and a decode proof artifact per recovery event. |
| **Galaxy-brain card** | A compact, human-auditable summary of an `EvidenceLedger` entry: action chosen, the prior/posterior pair, the top evidence terms, and the expected losses for each alternative action. `decision_to_card(record)` converts a ledger row to one. |
| **Conformance packet** | A single JSON file under `crates/fp-conformance/fixtures/packets/` (e.g. `fp_p2d_079_series_take_negative_indices_strict.json`) containing the input fixture, the operation to run, the oracle-pinned expected output, and `fixture_provenance` metadata. Parity reports and RaptorQ sidecars live in `artifacts/phase2c/`. 1,252 packet files are tracked today. |
| **Parity gate** | A green/red CI check that runs every conformance packet against the live pandas oracle (or fixture replay when offline) and refuses to merge if any gate flips red. Aggregate counts (ran/skipped/failed) are surfaced through `fp-ci-gates`. |
| **Phase 2C drift ledger** | `artifacts/phase2c/drift_history.jsonl`, an append-only log of parity-report deltas over time. Used to track regressions and confirm that fixes don't re-introduce earlier divergences. |
| **Compat-closure attestation pack** | A bundle (RaptorQ-protected) of all compat-closure evidence packs for a release candidate. Refreshed on every conformance gate run. Lives under `artifacts/phase2c/compat_closure/`. |
| **MultiIndexOrIndex** | The algebraic enum that flows through every public DataFrame API where either a flat `Index` or a `MultiIndex` can stand. Distinguishes the *logical* row axis (`row_multiindex` metadata when set) from the *flat storage* index used internally for direct row addressing. |
| **DuplicateLabelFlag** | Persisted `Option<bool>` on each DataFrame mirroring pandas' `df.flags.allows_duplicate_labels`. Read via `df.allows_duplicate_labels()`, set via `df.set_flags(Some(false))`. |
| **Hardened mode** | Runtime policy that accepts ambiguous inputs but routes them through a Bayesian decision (Allow / Reject / Repair) that minimizes expected loss. Compare with Strict mode which fails fast on any ambiguity. |

## Bayesian Decision Matrix in Detail

When `RuntimePolicy::Hardened` is active and the runtime encounters an ambiguous input (an unknown protocol field, a join admission outside the allowlist, an overflow-risk numeric) it constructs a `DecisionRecord` via Bayesian expected-loss minimization. The pieces:

**`IssueKind`** is the four-variant enum of issue classes the runtime currently distinguishes (`crates/fp-runtime/src/lib.rs`):

| Variant | Meaning |
|---------|---------|
| `UnknownFeature` | A protocol field, option, or capability that the strict allowlist doesn't recognize |
| `MalformedInput` | Inputs that violate dtype / null / shape preconditions detectable at the IO or parse boundary |
| `JoinCardinality` | A join admission that would produce more output rows than the policy cap allows |
| `PolicyOverride` | An explicit operator override of a defaulted policy decision |

**Priors** calibrated for the two priors with named constants in source:

| Issue class | `P(compatible)` | Source |
|-------------|-----------------|--------|
| `UnknownFeature` | 0.25 | `UNKNOWN_FEATURE_PRIOR` |
| `JoinCardinality` | 0.60 | `JOIN_ADMISSION_PRIOR` |

`MalformedInput` and `PolicyOverride` use a default conservative prior in code paths that construct an `EvidenceLedger` entry inline. Additional priors can be plumbed by callers via the explicit `DecisionRecord::with_prior` builder.

**Evidence terms** are `EvidenceTerm { name, log_likelihood_if_compatible, log_likelihood_if_incompatible }`. The canonical built-in term names ship as `Cow<'static, str>` constants in `fp-runtime`; callers can construct ad-hoc terms with any `name`. Representative terms used in the codebase:

| Term | compat | incomp | Where it fires |
|------|--------|--------|----------------|
| `compatibility_allowlist_miss` | −3.5 | −0.2 | Strict-mode allowlist did not include this subject |
| `unknown_protocol_field` | −2.0 | −0.1 | An IO format saw an option/key it does not recognize |
| `estimator_overflow_risk` | −0.3 | −1.2 | A numeric overflow predicate fired (e.g. integer mul or accumulation) |
| `memory_budget_signal` | varies | varies | Estimated working set exceeds the configured arena budget |

Terms are summed against the prior in log-space to produce the posterior via Bayes' theorem.

**Default loss matrix** (`LossMatrix::default()` in source; asymmetric, with false-allow priced 200× higher than false-reject-when-incompatible):

| | True compat | True incomp |
|---|---|---|
| Action: Allow | 0.0 | **100.0** |
| Action: Reject | 6.0 | 0.5 |
| Action: Repair | 2.0 | 3.0 |

A second `LossMatrix` (`JOIN_ADMISSION_LOSS`) is used for join-cardinality decisions with a heavier `allow_if_incompatible=130.0` penalty. Each decision picks `action = argmin E[loss]` where `E[loss(a)] = L(a | compat) · P(compat) + L(a | incomp) · P(incomp)`.

See the **EvidenceLedger Wire Format** section below for a complete example of a serialized `DecisionRecord`. Every entry is reproducible: the prior + evidence vector + loss matrix uniquely determine the action, so any third party can replay a ledger and verify the runtime's decisions.

## Memory Model: Bytes Per Cell, Per Column, Per DataFrame

FrankenPandas' generic `Vec<Scalar>` storage is more expensive than pandas' numpy-backed primitives. Concrete numbers help when sizing fleets:

| dtype | Scalar variant size | Per-cell overhead (Scalar) | Per-cell validity overhead | Hot-path typed view (AG-10) |
|-------|---------------------|-----------------------------|-----------------------------|------------------------------|
| `Float64` | 16 bytes (tag + 8B payload + padding) | 16 B | 1 bit | `&[f64]` direct, SIMD-friendly |
| `Int64` | 16 bytes | 16 B | 1 bit | `&[i64]` direct |
| `Bool` | 16 bytes | 16 B | 1 bit | `&[bool]` direct (1B/elem) |
| `Utf8` | 24 bytes + heap | 24 B + string bytes | 1 bit | `&[String]` direct |
| `Datetime64` | 16 bytes (i64 nanos) | 16 B | 1 bit | `&[i64]` direct |
| `Timedelta64` | 16 bytes (i64 nanos) | 16 B | 1 bit | `&[i64]` direct |
| `Categorical` | 16 bytes (Int64 code) + shared `CategoricalMetadata` | 16 B + amortized categories | 1 bit | `&[i64]` codes |
| `Period` *(value type, in PeriodIndex)* | 24 bytes (Period struct) | 24 B | 1 bit (when wrapped in nullable container) | `&[Period]` direct on PeriodIndex |
| `Interval` *(value type, in IntervalIndex)* | 40 bytes (start + end + closed) | 40 B | 1 bit (when wrapped in nullable container) | `&[Interval]` direct on IntervalIndex |
| `Null(NullKind)` | 16 bytes | 16 B | 1 bit (= 0) | n/a |

For a Float64 column with 10M rows: ~152 MB on the heap (≈160 MB / 10M ≈ 16 B/elem, plus ~1.25 MB validity), versus ~80 MB for a pandas numpy `float64` array. The constant-factor cost buys uniform null handling, dtype-erased generic kernels, and the ability to materialize an AG-10 typed view (`ColumnData::Float64(Vec<f64>)`) on hot paths so SIMD still kicks in.

**`ValidityMask`** itself uses ≤ 1 bit per element, rounded up to the next `u64` word. For 10M elements: 1.25 MB versus pandas' 8-byte nullable-extension type (80 MB).

**Index storage**: `Vec<IndexLabel>` where each `IndexLabel` is one of `Int64(i64)`, `Utf8(String)`, `Timedelta64(i64)`, `Datetime64(i64)`. Those are the four current variants. For 10M `Int64` row labels: ~152 MB (same Scalar-style overhead). For a `RangeIndex(start, stop, step)`, materialization is lazy: three `i64`s total regardless of length. `PeriodIndex` and the various typed-Index variants wrap their own value types in `Vec<…>`; they're not stored as `IndexLabel` variants.

**MultiIndex** adds `Vec<Vec<IndexLabel>>` (one level vector per level) plus a `Vec<Option<String>>` for the level names; for a 2-level Int64 MultiIndex over 10M rows: ~300 MB.

**Per-operation arena overhead**: GroupBy and Join with the Bumpalo path allocate one slab up front (default 256 MB) for all intermediates. The arena is dropped wholesale at the end; there's no per-group `free()` cost.

## Performance Tuning Playbook

| Situation | Recommended path | Why |
|-----------|------------------|-----|
| `groupby([single_int_col])` with `key_max - key_min ≤ 65,536` | Dense Int64 (automatic) | O(1) per element, zero hash overhead. Common for year/month/category-id keys. |
| `groupby([key])` with cardinality ≤ ~10M and total working set ≤ arena budget | Arena-backed Bumpalo (automatic) | Single `malloc`, pointer-bump pushes, bulk dealloc. Cache-friendly. |
| `groupby([key])` with unbounded cardinality or Utf8 keys | HashMap with typed `ScalarKey` (automatic fallback) | Stores `(source_index, accumulator)` pairs; never clones the key Scalar. |
| Adjusting the arena budget | Set `ExecOptions::arena_budget_bytes` on the `ExecOptions` you pass to `fp-groupby` / `fp-join` (or override the default that DataFrame's high-level entry points read) | Default 256 MB. Increase when you know the working set is large; decrease in memory-constrained environments to force the HashMap path sooner. |
| Many DataFrame-to-DataFrame ops on identically-indexed frames | Use shared `Index` values (build once, clone the Arc) | AG-11 identity-alignment fast path skips the alignment planner entirely when both operands share an Index with no duplicates. The `has_duplicates()` check is O(1) after the first call via `OnceLock` memoization. |
| Many lookups on the same sorted Index | Build the Index, call `position()` repeatedly | The first call detects sort order and caches it in `OnceLock<SortOrder>`. Subsequent calls dispatch directly to binary search (O(log n)) instead of HashMap construction. |
| Bulk `value_counts` / `nunique` / `mode` on string columns | Already O(n) via HashMap-keyed paths (2026-05 sweep) | No tuning needed; the older O(n²) paths have been replaced. |
| Approximate distinct counts on huge cardinalities | Reach for `fp_groupby::approx_nunique` (or `HyperLogLog::new(precision)`) directly | HyperLogLog with rho-bit-sentinel fix; default `p=14` gives ~0.8% standard error at ~16 KB memory regardless of input cardinality. |
| Multi-way `from_series([s1, s2, ..., sN])` | Just call it; AG-05 leapfrog triejoin kicks in automatically | Single O(n log n) sorted-merge pass instead of iterative pairwise. |
| Mixing `Int64` + `Float64` arithmetic | Type up front via `astype(DType::Float64)` if you know the result will be Float64 anyway | Avoids per-element scalar fallback; lets AG-10 dispatch to `&[f64]` typed-array vectorization with SIMD auto-vectorization. |
| Profiling a hot path | Enable the `tracing` feature, plumb a subscriber, and look at the spans on `groupby`/`rolling`/`resample`/IO | Off by default to avoid the overhead. |

## HyperLogLog for Approximate Cardinality

`fp-groupby` exposes a HyperLogLog primitive for approximate distinct-count estimation when exact `nunique` would dominate memory:

```rust
use fp_groupby::{HyperLogLog, approx_nunique, SketchResult};

// One-shot convenience: returns a SketchResult with { value, error_bound, memory_bytes }.
let sk: SketchResult = approx_nunique(values);

// Lower-level: pick your precision (p), feed values yourself.
let mut hll = HyperLogLog::new(14);                   // 2^14 = 16,384 registers
for v in values.iter() { hll.insert(v); }
let estimate: f64 = hll.estimate();
// p=14 → ~1.04 / sqrt(16384) ≈ 0.8% standard error
//      → ~16 KB memory regardless of input cardinality
```

The implementation uses the standard `rho(hash)` count-leading-zeros register update, plus a sentinel-bit fix discovered during fuzz testing: the older "unconditional OR" rho path could overflow `u8::MAX` on degenerate hashes and crash on debug builds. The fix conditions the update on `rho < register_capacity`. Tests cover boundary conditions (empty input, single-value input, all-identical-values input, near-`u64::MAX` hash payloads).

GroupBy uses HLL internally when an `approximate=true` flag is set on `nunique`; otherwise it falls back to the exact `HashMap<ScalarKey, ()>` path.

## End-to-End: How a `df.query("x > 5 and y < 10")` Call Actually Executes

A worked-out trace, end-to-end:

1. **Tokenization (fp-expr)**: The string `"x > 5 and y < 10"` is scanned into `[Ident("x"), GT, Number(5), And, Ident("y"), LT, Number(10)]`. Backtick-quoted identifiers and `@local`-prefixed locals are distinguished here.
2. **Parsing (fp-expr)**: A recursive-descent parser produces an `Expr` AST:
   ```text
   And(
     Compare(GT, Column("x"), Literal(Number(5))),
     Compare(LT, Column("y"), Literal(Number(10))),
   )
   ```
   Chained comparisons (e.g. `0 < x < 10`) parse to a pairwise AND form to match pandas.
3. **Context resolution**: `EvalContext::from_dataframe(&df)` makes column references resolvable by name. `@local` variables are looked up in the supplied `BTreeMap` and broadcast to a Series of the right length.
4. **Evaluation (fp-expr)**: A bottom-up walk of the AST. Each Column reference fetches the column's `ColumnData` (the AG-10 typed view); each Literal is wrapped in a 1-element `Scalar`; each Compare dispatches to `vectorized_binary_*` with a `bool` output ValidityMask; each And does word-level `and_mask` on the validity bitmaps and a `Vec<bool> AND Vec<bool>` on the data words.
5. **Boolean mask result**: The final `Expr` evaluates to a `Series<bool>` with the same length and index as `df`. Nulls are propagated: if either side of `>` had a null at position `i`, the result at `i` is null (which `filter_rows` treats as `false`).
6. **Filter materialization (fp-frame)**: `df.filter_rows(&bool_mask)` walks each column once and assembles a new `DataFrame` with only the rows where the mask is `true`. The new DataFrame's `Index` is built from the filtered subset of the original.
7. **Index-name propagation**: The result inherits `df.index().name()` (and `df.row_multiindex` if set). This was the focus of the 2026-05 fork-wide sweep.
8. **EvidenceLedger entry**: If `RuntimePolicy::Hardened` is active, the runtime logs the filter as a `Repair` (no compatibility issues encountered) with prior `1.0`, posterior `1.0`, and an empty evidence-term vector. That's the no-op happy path. If a column reference had been ambiguous (e.g. duplicate column names), a real decision record would land in the ledger.

Total round-trip cost: one parse (microseconds for typical expressions), one Series allocation for the mask, one DataFrame allocation for the filtered result. No temporary DataFrames in between.

## Live Pandas Oracle: How It Works

The `fp-conformance` harness performs **differential testing**: it runs the same operation in FrankenPandas and in actual pandas (Python subprocess), then asserts that the outputs are bitwise-equivalent at the JSON level.

```
                  cargo test -p fp-conformance
                                │
                                ▼
                ┌──────────────────────────────┐
                │  PacketRunner (Rust thread)  │
                └──────┬────────────────┬──────┘
                       │                │
       ┌───────────────▼──┐      ┌──────▼─────────────────┐
       │ frankenpandas    │      │ pandas_oracle.py        │
       │   runs op (Rust) │      │   (Python subprocess)   │
       │                  │      │                         │
       │   writes JSON    │      │   stdin: input fixture  │
       │   to stdout      │      │   stdout: result JSON   │
       └──────┬───────────┘      └──────┬─────────────────┘
              │                          │
              ▼                          ▼
       ┌─────────────────────────────────────┐
       │  PacketComparator                    │
       │   - dtype equality                   │
       │   - value equality (semantic_eq)     │
       │   - row order                        │
       │   - index labels + names             │
       │   - column order                     │
       │   - null propagation                 │
       └─────────────┬───────────────────────┘
                     │
            ┌────────┴─────────┐
            ▼                  ▼
     parity_report.json    parity_gate_result.json
     (per-case detail)     (green/red verdict)
```

The Python subprocess is invoked with a worker-thread on the Rust side that drains stdout/stderr in parallel; earlier versions deadlocked when the Python process buffered output past the pipe's high-water mark. Pandas version is pinned in `crates/fp-conformance/oracle/requirements.txt`; the fixture freshness gate (`scripts/check_fixture_freshness.sh`) fails closed if the pandas pin and the regenerated fixtures don't match.

When `/dp/frankenpandas/legacy_pandas_code/pandas` doesn't exist (typical for non-author dev environments), the harness falls back to fixture replay: the previously-pinned oracle output is loaded from the packet directory and used as the reference. CI guarantees the live oracle ran on every PR via a system-pandas fallback.

A packet's on-disk layout is a **single flat JSON file** per packet, not a directory tree. Filenames are lowercased and underscore-separated:

```
crates/fp-conformance/fixtures/packets/
├── fp_p2c_001_series_add_strict.json
├── fp_p2c_001_series_add_alignment_union_strict.json
├── fp_p2c_005_groupby_sum_order_strict.json
├── fp_p2d_014_dataframe_merge_inner_strict.json
├── fp_p2d_079_series_take_negative_indices_strict.json
└── …~1,252 packet files plus an adversarial / smoke / perf-budget side-set
```

Each file's top-level keys are `packet_id`, `case_id`, `mode` (`"strict"` / `"hardened"`), `operation` (a snake_case string naming the `FixtureOperation` variant, e.g. `"series_add"`, `"series_take"`, `"groupby_sum"`), `fixture_provenance` (pandas version + oracle script SHA + generated timestamp), the input fixture(s) under op-specific keys (most ops use `left` / `right`; some have op-specific keys like `take_indices` for `series_take`), and `expected_*` (the pandas-oracle-pinned reference output). Per-run diagnostics (`parity_report.json`, `parity_gate_result.json`) and the RaptorQ sidecar are emitted to `artifacts/phase2c/` rather than rewritten into the packet file itself, keeping packet files diff-clean and review-friendly.

The 1,252 packet JSON files under `fixtures/packets/` exhaustively cover: alignment, join, concat, filter, CSV/JSON/Parquet/Excel/Feather/Arrow IPC round-trips, dtype invariants, null semantics, resample, rolling/expanding/ewm, groupby aggregates, datetime/string/timedelta accessors, MultiIndex round-trips, and IO error parity.

## A Tour Through `fp-frame` (the 87,000-line crate)

`fp-frame` is the load-bearing crate. The file is too large to skim linearly. Use `rg` to navigate; the rough offsets below are approximate orientation aids, verified against the 2026-05-16 layout but not promised to stay stable:

| Approximate offset | What's there |
|--------------------|--------------|
| Top of `lib.rs` (≈1–300) | Module-level docs, error types (`FrameError`), `Series` struct definition, basic constructors |
| `~2k` | `Series::new` and core accessors (`name`, `index`, `column`, `dtype`) |
| `~6k–10k` | Series arithmetic, comparison, `case_when`, `apply_fn`, `map_values`, Series-level groupby |
| `~10k–15k` | Series I/O bridging, Arrow interop |
| `~15k–18k` | `CategoricalAccessor`, `SparseAccessor`, `DatetimeAccessor`, `TimedeltaAccessor` (accessor return types live around `15k–18k`) |
| `~21k+` | `DataFrame` struct + base impl |
| `~22k–23k` | DataFrame constructors (`from_dict`, `from_records`, `from_tuples`, `from_series`, `from_csv`, `set_index_multi`) |
| `~23k–26k` | DataFrame selection, indexing, sorting, sampling, `to_dict`, `to_records`, `to_numpy_2d` |
| `~26k–28k` | DataFrame reshape (melt, pivot_table, stack, unstack, crosstab, explode) |
| `~28k–29k` | DataFrame statistical methods (`describe`, `corr`, `cov`, `nlargest_*`, `value_counts_subset`, `compare`) |
| `~29k–30k` | DataFrame IO-extension trait dispatch (`to_csv`, `to_json`, `to_excel`, `to_parquet`, `to_sql`, `to_html`, `to_latex`, `to_markdown`, `to_xml`, `to_pickle`, `to_stata`, `to_orc`, `to_hdf`, `to_xarray`) |
| `~30k–37k` | DataFrame arithmetic (sub_df / mul_df / div_df / pow_df / mod_df / floordiv_df + scalar variants), apply/transform/pipe, where/mask family |
| `~37k–55k` | Window/resample plumbing (Rolling, Expanding, Ewm, Resample structs + all their reductions) and GroupBy (DataFrame-level + Series-level) with `agg_named` / `agg_multi` |
| `~55k–87k` | Inline `#[cfg(test)]` modules (unit tests + proptest blocks) |

If you're searching for a specific pandas method:
- Start with `rg -nE "pub fn <method_name>\b" crates/fp-frame/src/`
- Or `rg -nE "fn <method_name>\b\s*\(" crates/fp-frame/src/`
- For accessor methods, append the receiver type: `rg -nE "impl DatetimeAccessor" crates/fp-frame/src/`

The offsets shift every time a major section lands; use them as starting points, not as load-bearing references.

## Migration Guide: pandas (Python) → FrankenPandas (Rust)

Until PyO3 bindings ship, FrankenPandas is a Rust library. The cookbook below maps the most common pandas patterns to their FrankenPandas equivalents. Note that error handling moves into the type system: everything that pandas raises becomes a `Result<_, _>`.

### Construction

```python
# pandas
import pandas as pd
df = pd.DataFrame({"a": [1, 2, 3], "b": ["x", "y", "z"]})
```

```rust
// FrankenPandas — note from_dict_mixed takes a column_order slice + Vec<(col, input)>
use frankenpandas::prelude::*;

let df = DataFrame::from_dict_mixed(
    &["a", "b"],
    vec![
        ("a", DataFrameColumnInput::from_iter([1i64.into(), 2i64.into(), 3i64.into()])),
        ("b", DataFrameColumnInput::from_iter(["x".into(), "y".into(), "z".into()])),
    ],
)?;
```

### Selection

| pandas | FrankenPandas |
|--------|---------------|
| `df["a"]` | `df.column("a").unwrap()` (returns `&Column`); or `Series::new(...)` to wrap as Series |
| `df[["a", "b"]]` | `df.select_columns(&["a", "b"])?` |
| `df.loc[label]` | `df.loc(&[label.into()])?` |
| `df.iloc[i]` | `df.iloc(&[i as i64])?` |
| `df.iloc[-1]` | `df.iloc(&[-1])?` (negative indexes resolve from the end) |
| `df.at[label, col]` | `df.at(&label.into(), col)?` |
| `df.iat[i, j]` | `df.iat(i as i64, j as i64)?` (both args are `i64`) |
| `df.head(10)` | `df.head(10)?` |
| `df.tail(-5)` | `df.tail(-5)?` (negative `n` keeps all but the first 5) |
| `df.xs("a")` | `df.xs(&"a".into())?` |

### Filtering

| pandas | FrankenPandas |
|--------|---------------|
| `df[df.x > 5]` | `df.query("x > 5")?` (preferred; uses the expression engine) |
| `df.query("x > 5 and y < 10")` | `df.query("x > 5 and y < 10")?` (literally the same string) |
| `df[df.x.isin([1, 2, 3])]` | `let mask = df.column("x").unwrap().isin(&[1i64.into(), 2i64.into(), 3i64.into()])?; df.filter_rows(&mask)?` |
| `df.dropna()` | `df.dropna()?` |
| `df.fillna(0)` | `df.fillna(&Scalar::Float64(0.0))?` |

### Aggregation

| pandas | FrankenPandas |
|--------|---------------|
| `df.groupby("region").sum()` | `df.groupby(&["region"])?.sum()?` |
| `df.groupby("region").agg({"x": "sum", "y": "mean"})` | `df.groupby(&["region"])?.agg_named(&[("x", "x", "sum"), ("y", "y", "mean")])?` |
| `df.groupby(["a", "b"]).size()` | `df.groupby(&["a", "b"])?.size()?` |
| `df.describe()` | `df.describe()?` |
| `df.corr()` | `df.corr()?` |
| `df.value_counts("region")` | `df.value_counts_subset(&["region"])?` |

### Reshape

| pandas | FrankenPandas |
|--------|---------------|
| `df.pivot_table(values="x", index="r", columns="c", aggfunc="sum")` | `df.pivot_table("x", "r", "c", "sum")?` |
| `df.melt(id_vars=["a"], value_vars=["b", "c"])` | `df.melt(&["a"], &["b", "c"], None, None)?` |
| `df.stack()` | `df.stack()?` |
| `df.unstack()` | `df.unstack()?` |
| `pd.get_dummies(df, columns=["color"])` | `df.get_dummies(&["color"])?` |
| `pd.crosstab(df.a, df.b)` | `let a = ...; let b = ...; DataFrame::crosstab(&a, &b)?` |
| `df.explode("tags")` | `df.explode("tags", ",")?` (sep is required; pandas auto-detects list-cells, we ask for the explicit separator) |

### IO

| pandas | FrankenPandas |
|--------|---------------|
| `pd.read_csv("file.csv")` | `read_csv(Path::new("file.csv"))?` (or `read_csv_with_options_path(&path, &CsvReadOptions::default())?` for the full option matrix) |
| `pd.read_csv(io.StringIO(s))` | `read_csv_str(s)?` |
| `pd.read_excel("file.xlsx", sheet_name="Sheet1")` | `read_excel_bytes(&bytes, &ExcelReadOptions { sheet_name: Some("Sheet1".to_owned()), ..Default::default() })?` |
| `df.to_csv("file.csv", index=False)` | In-memory form: `let s = df.to_csv_options(',', /* include_index */ false, "", None)?;` then write `s` to disk. The free `write_csv(&df, Path::new("file.csv"))?` is index-aware; the options matrix lives on `write_csv_string_with_options(&df, &CsvWriteOptions { index: false, .. })?`. |
| `df.to_parquet("file.parquet")` | `df.to_parquet(Path::new("file.parquet"))?` |
| `df.to_sql("table", conn, if_exists="replace")` | `df.to_sql(&conn, "table", SqlIfExists::Replace)?` (the simple `to_sql` takes the `if_exists` enum directly; for the pandas-style options matrix use `df.to_sql_with_options(&conn, "table", &SqlWriteOptions { if_exists: SqlIfExists::Replace, ..Default::default() })?`) |
| `pd.read_sql("SELECT * FROM t", conn)` | `read_sql_with_options(&conn, "SELECT * FROM t", &SqlReadOptions::default())?` (note arg order is `(conn, query, opts)`) |

### Datetime

| pandas | FrankenPandas |
|--------|---------------|
| `pd.to_datetime(s)` | `to_datetime(&s)?` |
| `pd.to_datetime(s, format="%d-%b-%Y")` | `to_datetime_with_format(&s, Some("%d-%b-%Y"))?` |
| `s.dt.year` | `s.dt().year()?` |
| `s.dt.tz_localize("UTC")` | `s.dt().tz_localize(Some("UTC"))?` |
| `pd.date_range("2024-01-01", "2024-12-31", freq="M")` | `date_range(Some("2024-01-01"), Some("2024-12-31"), None, /* freq nanos */ MONTH_NS, None)?` (freq is `i64` nanoseconds, not a `&str` alias; use `PeriodFreq::parse` to convert from the pandas alias) |

### Apply

| pandas | FrankenPandas |
|--------|---------------|
| `df.apply(lambda r: r.x + r.y, axis=1)` | `df.apply_row("sum", \|row\| { row[0].clone() })?` |
| `df.applymap(f)` | `df.applymap(f)?` |
| `s.apply(f)` | `s.apply_fn(f)?` |
| `s.map({"a": 1, "b": 2})` | `s.map_values(&map)?` |
| `df.pipe(fn).pipe(fn2)` | `df.pipe(fn)?.pipe(fn2)?` |

## What's New in the 2026-05 Wave

The last ~750 commits (2026-05-02 → 2026-05-16) focused on three big themes, all detailed in `CHANGELOG.md` Phase 2c. The highlights:

1. **Fork-wide "preserve index name" sweep (~76 commits, 70+ callsites)**. Every helper method that produces a new Series or DataFrame now propagates the source axis name correctly. The retrofit covered Rolling, Expanding, Ewm, Resample, SeriesGroupBy, DataFrameGroupBy, plus 40+ Series methods and the StringMethods / DatetimeAccessor surface. A single commit (`fe61bbd3`) converted 36 transform call sites to a shared helper in one swoop.
2. **Typed-Index variant build-out (60+ methods on a single day)**. `DatetimeIndex`, `TimedeltaIndex`, `PeriodIndex`, `RangeIndex`, and `CategoricalIndex` each gained the full pandas method surface: time-of-day accessors, set ops, slice ops, `get_loc` / `get_indexer` family, `tz_localize` / `tz_convert`, `searchsorted`, `where` / `putmask`, `asof` / `asof_locs`, freq inference, mean/median/std/var, etc.
3. **IO format surface expansion**. One day added HTML, XML, LaTeX, Markdown writers, Pickle round-trip, Stata round-trip, ORC round-trip, HDF5 snapshot (optional feature), `read_table` (TSV), and `read_fwf` (with automatic colspec inference).
4. **Algorithmic complexity sweep**. ~20 separate O(n²) → O(n) reductions: `value_counts`, `unique`, `nunique_with_dropna`, `duplicated`, `drop_duplicates`, `map`, `replace`, `isin`, `cut`, `qcut`, `mode_with_dropna`, `DataFrame::nunique`, `DataFrame::value_counts`, `DataFrame::append` column union, `Series::drop`, `Series::unstack`, `str.get_dummies`, `factorize`, `DataFrame::get_dummies`, CSV NA HashSet, Excel sheet HashSet.
5. **Timedelta64 fast paths across the reduction family**. `nanmin`, `nanmax`, `nansum`, `nanmean`, `nancumsum`, `nancummin`, `nancummax`, `nanmedian`, `nanvar`, `nanstd`, `nansem`, `nanquantile`, `nanargmax`, `nanargmin`, `nanptp`, `nanprod`, Column `pct_change` / `diff`, Series `pct_change`, GroupBy `pct_change`, SeriesGroupBy `sum`/`mean`/`min`/`max`. Timedelta64 columns now produce the right type instead of silently coercing or panicking.
6. **DataFrame plotting / sparse / style / xarray / duplicate-label-flag capabilities**: plot specs (`PlotSpec` / `BoxPlotSpec`), `DataFrame::to_xarray()`, sparse `density()` / `npoints()` metrics on `SparseAccessor`, persisted `allows_duplicate_labels` flag, `StyledDataFrame` HTML rendering via `df.style()`, Series `list` accessor, Series `r#struct` accessor.

For full commit-level detail, see `CHANGELOG.md` Phase 2c (lines ~660 onward).

## How `fp-conformance` Is Wired Into CI

The CI pipeline runs the conformance harness in two modes on every PR:

1. **Live oracle mode** (when system pandas is installed in the CI image): every packet runs FrankenPandas + invokes a Python subprocess loading pandas, then compares outputs. The pandas pin lives in `crates/fp-conformance/oracle/requirements.txt`; the build fails closed if that pin and the regenerated fixtures don't agree.
2. **Fixture replay mode** (always, as a baseline): every packet runs FrankenPandas and compares against the previously-pinned oracle output stored in the packet directory.

Beyond the differential harness, CI also runs:

- `cargo fmt --check`: formatting hygiene
- `cargo clippy --workspace --all-targets -- -D warnings`: pedantic + nursery lints
- `cargo test --workspace`: all 5,000+ unit tests
- `cargo doc --workspace --no-deps` with `-D warnings`: rustdoc completeness
- `cargo audit`: security advisories
- `cargo deny check`: license/source policy
- `cargo machete`: unused dependency detection
- **Fuzz regression corpus**: every PR replays the regression seeds for all 30 fuzz targets
- **Perf baseline gate**: `cargo test -p fp-conformance --test perf_baselines -- --ignored` with budgets enforced

A green PR has passed all of these.

## A Note On `unsafe`

Every crate in the workspace begins with:

```rust
#![forbid(unsafe_code)]
```

This includes `fp-columnar` (where SIMD auto-vectorization happens via LLVM, not via `std::simd` intrinsics), `fp-types` (where the typed `Scalar` enum is dispatched at compile time, not via tagged pointers), and `fp-io` (where binary format readers like Parquet and Feather use Apache Arrow's `arrow-rs` crate, which keeps its `unsafe` internally; we never re-export or re-use those `unsafe` boundaries).

The build will fail if `unsafe` ever appears in a workspace crate. The closest analog to `unsafe` we ever reach for is `#[allow(clippy::cast_*_truncation)]` to silence pedantic lint warnings on intentional `i64 → i32` casts at IO boundaries.

## Acknowledgments

- **Apache Arrow** (`arrow-rs` + `parquet`) for the columnar interchange format that underpins Parquet, Feather, and Arrow IPC IO.
- **`calamine`** for the Excel reader/writer surface.
- **`rusqlite`** for the default SQL backend; the `SqlConnection` trait is generic enough to wrap any other backend that exposes a similar query/transaction interface.
- **`chrono-tz`** for the IANA time-zone database.
- **`bumpalo`** for the bump allocator used by the arena-backed GroupBy/Join path.
- **`proptest`** for the property-test framework that backs the 100+ invariants.
- **`libfuzzer-sys`** + `cargo-fuzz` for the 30 fuzz targets.
- **`hdf5-metno`** for the optional HDF5 backend.
- **`raptorq`** for the repair-symbol envelope around durable artifacts.
- **The pandas core team**, whose oracle we benchmark against on every commit. The clean-room reimplementation is only tractable because the original API surface is so consistently specified.

## ValidityMask: Boolean Algebra At The Word Level

The bitpacked null bitmap underpins every null-aware operation in FrankenPandas. The math is worth seeing concretely because it's the hottest path in the codebase.

**Layout** for `len = 130` elements:

```
words.len() == ceil(130 / 64) == 3

word 0:  bits 0..64    (elements 0..64)
word 1:  bits 64..128   (elements 64..128)
word 2:  bits 128..130  (elements 128..130; bits 130..192 are zeroed)
```

The "tail mask" on the last word ensures unused high bits never contaminate aggregate counts. Construction:

```
tail_bits     = len % 64                // 2 for len=130
tail_mask     = (1u64 << tail_bits) - 1 // 0b11
words[last]  &= tail_mask
```

**Boolean operations** combine two masks word-by-word:

```rust
// Pseudocode for and_mask (the real implementation lives in fp-columnar)
debug_assert_eq!(a.len, b.len);
let mut out = Vec::with_capacity(a.words.len());
for (wa, wb) in a.words.iter().zip(b.words.iter()) {
    out.push(wa & wb);
}
out[last] &= tail_mask;          // re-mask the tail
ValidityMask { words: out, len: a.len }
```

`or_mask` uses `|`, `not_mask` uses `!` (and re-applies the tail mask). De Morgan's law and the involution of `not` are both encoded as proptests in `fp-columnar`, run over hundreds of random length / pattern combinations.

**Counting valid bits** uses hardware POPCNT:

```rust
fn count_valid(mask: &ValidityMask) -> usize {
    mask.words.iter().map(|w| w.count_ones() as usize).sum()
}
```

For `len = 1_000_000`, that's 15,625 `POPCNT` instructions instead of a million per-element branches. On modern x86_64 silicon `POPCNT` retires in a single cycle.

**Bit-set / clear / test** on individual positions:

```rust
fn set(&mut self, idx: usize, value: bool) {
    let word = idx / 64;
    let bit  = idx % 64;
    if value { self.words[word] |=  (1u64 << bit); }
    else     { self.words[word] &= !(1u64 << bit); }
}

fn get(&self, idx: usize) -> bool {                   // canonical accessor name
    let word = idx / 64;
    let bit  = idx % 64;
    (self.words[word] & (1u64 << bit)) != 0
}
```

For bulk-iterating all bits (mostly used by tests and the `Display` impl), `ValidityMask::bits()` yields a `bool` per element. Hot-path operations that conceptually "visit only the non-null rows" almost always pair the mask with the data column and walk both together with the validity guarding the data load, rather than materializing the position list. That avoids one allocation and keeps the loop branch-predictable.

## ScalarKey: Dtype-Aware Hashing

The naive way to use `Scalar` as a `HashMap` key fails for several reasons:

1. `Float64(0.0)` and `Float64(-0.0)` are bit-distinct but semantically equal.
2. `Float64(NaN)` is not equal to itself by IEEE 754, but groupby treats it as a single group.
3. `Int64(1)` and `Float64(1.0)` are not the same key (pandas treats their groups separately).
4. `Scalar` itself doesn't implement `Hash` because `Float64` isn't `Eq` (NaN ≠ NaN).

`ScalarKey` is the canonical key type that fixes all four. It's a **private, lifetime-parameterized** enum (one copy in `fp-types::nannunique`, one in `fp-frame`), with six variants:

```rust
// crates/fp-frame/src/lib.rs — schematic, not the exact ordering/derives
enum ScalarKey<'a> {
    Null(NullKind),                // collapses all three NullKinds
    Bool(bool),
    Int64(i64),
    FloatBits(u64),                // canonicalized: -0.0 → +0.0 before to_bits()
    Utf8(&'a str),                 // borrowed; no clone on insert
    Timedelta64(i64),
}
```

There's no `impl From<Scalar>`; conversion goes through two helper free functions:

```rust
// Either keep nulls as a real group key, or skip them at construction time.
scalar_key_allow_missing(scalar: &Scalar) -> ScalarKey<'_>
scalar_key_skip_missing(scalar: &Scalar) -> Option<ScalarKey<'_>>
```

The float canonicalization:

```rust
Scalar::Float64(v) => {
    let normalized = if *v == 0.0 { 0.0 } else { *v };  // collapse -0 → +0
    ScalarKey::FloatBits(normalized.to_bits())
}
Scalar::Null(kind) => ScalarKey::Null(*kind),           // NaN-bearing Floats go here too
```

(`Datetime64` and `Period`/`Interval` Series store their codes as `Int64` or `Utf8` Scalars, so they round-trip through the existing variants without needing dedicated ScalarKey arms.)

Replacing the older `format!("{val:?}")` string keys with `ScalarKey` (commit `1b52ae43` on 2026-04-11) closed a long tail of subtle parity bugs: `groupby` on a Float64 column with both `0.0` and `-0.0` had been creating two groups instead of one; `mode` on a column with NaNs occasionally returned different counts depending on debug-print formatting; and dtype-collision bugs where `Int64(1)` and `Float64(1.0)` should have been separate groups but weren't.

## How GroupBy Picks Its Execution Path (Code Walkthrough)

The decision tree from `fp-groupby/src/lib.rs`, in plain English. (This isn't a single `choose_path` function; each `groupby_<agg>_with_trace` entry point reproduces the same inline logic.)

```
For each call to groupby_<sum|mean|min|max|...>_with_trace(input, by, exec_options):

    if (agg == "sum") AND                                       // dense path is sum-only today
       all by-columns have dtype Int64 AND
       no by-column has nulls AND
       (max(key) - min(key)) <= DENSE_INT_KEY_RANGE_LIMIT       // const 65_536
        : run try_groupby_sum_dense_int64() and return.

    estimated_bytes = estimate_intermediates(input, by)
    if exec_options.use_arena && estimated_bytes <= exec_options.arena_budget_bytes
        : execute on the arena-backed Bumpalo path.

    else
        : execute on the global-allocator HashMap path.
```

`GroupByExecutionOptions` (the actual struct name, in `fp-groupby/src/lib.rs`) has just two configurable fields:

```rust
pub struct GroupByExecutionOptions {
    pub use_arena: bool,                 // default true
    pub arena_budget_bytes: usize,       // default DEFAULT_ARENA_BUDGET_BYTES = 256 * 1024 * 1024
}
```

The dense-Int64 cutoff (`DENSE_INT_KEY_RANGE_LIMIT = 65_536`) is a module-level constant, not a configurable field; it's tuned to the size of L2 cache on representative server hardware.

The three paths share a common output assembly stage that materializes a result `DataFrame` from `(group_key, accumulator)` pairs. The arena path differs only in how its intermediates are allocated; the global path differs only in the allocator and absence of bulk-dealloc.

**The dense path** materializes a fixed-size `Vec<Accumulator>` of length `(max_key - min_key + 1)`. Every element starts in the "empty accumulator" state; per-row processing is:

```rust
for (key, value) in keys.iter().zip(values.iter()) {
    let slot = (key - min_key) as usize;
    accumulators[slot].push(value);  // O(1) array write
}
```

Group-key reconstruction at output time is trivial: index `i` corresponds to `min_key + i`, and only slots whose accumulator is non-empty are emitted.

**The arena path** uses a per-operation `Bumpalo` allocator. Each new group's `Accumulator` is bump-allocated from the arena. Hash collisions are resolved by linear probing within an arena-allocated open-addressed table. At the end, the arena is dropped wholesale; no per-group `free()`.

**The global-allocator path** uses `std::collections::HashMap<ScalarKey, Accumulator>` directly. It's the slowest of the three because of the per-group `Box<Accumulator>` allocations, but it's the only path that handles arbitrary key types and unbounded cardinality without an a-priori memory bound.

**Property test:** for every random `(keys, values, by_columns)` input under proptest, the three paths' outputs are asserted bitwise-equal. This is one of the strongest correctness guarantees in the codebase; the dense path's behavior under high-cardinality keys is automatically checked against the safer HashMap fallback.

## Leapfrog Triejoin (AG-05) in Detail

The N-way alignment problem: given `N` indexes `I_1, I_2, ..., I_N`, compute the union (or intersection) in a single pass.

The pairwise approach is `O(N · max_len · log max_len)` and re-builds the union HashMap N times. The leapfrog triejoin trades one merge for one comparison per element:

```
1. Sort each input index once. (Or detect that it's already sorted via AG-13.)
2. Maintain a cursor (position) into each input.
3. At each step:
     a. Look at the labels at all N cursors.
     b. Pick the smallest (for union) or the maximum (for intersection).
     c. For union: emit it, advance every cursor whose label equals the smallest.
     d. For intersection: if all cursors have the same label, emit it and advance all;
        otherwise, advance the cursor with the smallest label.
4. Stop when any cursor falls off the end (intersection) or all fall off (union).
```

Worst-case complexity: `O((N + total_unique_labels) · log N)` from the priority-queue-style smallest-element selection. For typical `N=2..5` and indexes of length `10^4..10^7`, this runs noticeably faster than the iterative pairwise approach.

The implementation lives in `fp-index::multi_way_align`. `DataFrame::from_series([s1, s2, s3])` routes through it automatically; manual callers can also invoke `leapfrog_intersection` / `leapfrog_union` directly.

## Conformance Packet Authoring Guide

Adding a new packet to the conformance corpus (1,252 and counting):

1. **Pick a packet ID**: increment from the latest `fp_p2d_NNN_*.json` (DataFrame surface) or `fp_p2c_NNN_*.json` (Series surface) in `crates/fp-conformance/fixtures/packets/`. The `bv --robot-triage` and `br ready` workflows surface coverage gaps; the `scripts/gen_pandas_api_listing.py`, `scripts/gen_coverage_matrix.py`, and `scripts/gen_feature_parity_table.py` reports flag pandas APIs with no packet yet.
2. **Create the single packet JSON file**, e.g. `crates/fp-conformance/fixtures/packets/fp_p2d_434_dataframe_my_new_op_strict.json`. The file's top-level keys are `packet_id`, `case_id`, `mode` (`"strict"` / `"hardened"`), `operation` (a snake_case string that deserializes into a `FixtureOperation` variant), `fixture_provenance` (pandas version + oracle script SHA + generated timestamp), the input fixture(s) keyed by role (typical: `left` / `right`; some ops carry op-specific top-level keys like `take_indices` for `series_take`), and `expected_*` (the pandas-oracle-pinned reference output). Example skeleton:
   ```json
   {
     "packet_id": "FP-P2D-434",
     "case_id": "dataframe_my_new_op_strict",
     "mode": "strict",
     "operation": "dataframe_my_new_op",
     "fixture_provenance": {
       "pandas_version": "2.2.3",
       "oracle_script_sha256": "…",
       "generated_at": "2026-05-17T00:00:00Z"
     },
     "left": { "name": "lhs", "index": [{"kind":"int64","value":1}, {"kind":"int64","value":2}],
               "values": [{"kind":"int64","value":10}, {"kind":"int64","value":20}] },
     "expected_frame": { /* … */ }
   }
   ```
   Valid `operation` strings are the snake_case forms of the `FixtureOperation` variants (in `crates/fp-conformance/src/lib.rs`); add a new variant there if you need a new op.
3. **Generate the expected output** by running `crates/fp-conformance/oracle/pandas_oracle.py` against your input under the pinned pandas version (`crates/fp-conformance/oracle/requirements.txt`). The oracle script writes the expected output back into the packet JSON.
4. **Run the gate**: `./scripts/phase2c_gate_check.sh`. Confirms the packet runs green against both the fixture-replay and live-oracle paths, and that the aggregated parity reports under `artifacts/phase2c/` pick up the new packet.
5. **Verify fixture freshness**: `./scripts/check_fixture_freshness.sh`. Fails closed if the packet was regenerated against a pandas version that doesn't match the pin in `requirements.txt`.
6. **Wire the operation** into the dispatch in `crates/fp-conformance/src/lib.rs` if it's a new op kind; existing kinds just need the fixture file.
7. **Document any divergence**: if the new packet's expected output differs from FrankenPandas' actual behavior, add a `DISC-NNN` entry to `crates/fp-conformance/DISCREPANCIES.md` with root-cause analysis and resolution status (ACCEPTED / INVESTIGATING / WILL-FIX).
8. **Commit**: include the packet file, any dispatch change, any DISCREPANCY entry, and the bead closeout in one commit. Use `br close <id>` to close the related Beads issue.

The fixture freshness gate (`scripts/check_fixture_freshness.sh`) runs in CI and fails closed if the pandas pin in `oracle/requirements.txt` doesn't match the regenerated fixtures' provenance.

## Reading the Source: Starter Trails

Concrete first reads for someone new to the codebase, by area of interest.

**If you're interested in alignment / AACE:**
- `crates/fp-index/src/lib.rs`: start at `align_union`, then `align_inner`, `align_left`, `align_non_unique`. Read `AlignmentPlan` and the `validate_alignment_plan` debug-mode assert.
- `crates/fp-index/src/lib.rs`: `multi_way_align`, the leapfrog triejoin entry point.
- `crates/fp-frame/src/lib.rs`: search for `align_union` to see the call sites. Series arithmetic and DataFrame.add_df are the canonical examples.

**If you're interested in the runtime decision layer:**
- `crates/fp-runtime/src/lib.rs`: start at `RuntimePolicy`, then `DecisionAction`, `IssueKind`, `LossMatrix`, and the (private) `decide` function. Read `EvidenceLedger::push` + `records()` for how decisions are persisted and replayed.

**If you're interested in IO formats:**
- `crates/fp-io/src/lib.rs`: start at the `DataFrameIoExt` trait. Each format has its own `read_*` + `write_*` pair. Search for `pub fn read_csv` and follow.
- For SQL: start at the `SqlConnection` trait, then the `SqlInspector` wrapper, then the `impl SqlConnection for rusqlite::Connection` block (the bundled backend; other backends just implement the same trait).

**If you're interested in the type system:**
- `crates/fp-types/src/lib.rs`: start at `DType`, `Scalar`, `NullKind`, `common_dtype`, `cast_scalar`. Read the `Timestamp` / `Timedelta` / `Period` / `Interval` struct definitions.

**If you're interested in GroupBy:**
- `crates/fp-groupby/src/lib.rs`: start at `GroupByExecutionOptions`, then the per-aggregation `groupby_<agg>_with_trace` entry points (each carries the dense/arena/HashMap path selection inline), then the sum-only fast path `try_groupby_sum_dense_int64`.
- `crates/fp-frame/src/lib.rs`: search for `pub fn groupby` to see how DataFrame and Series-level groupby dispatch.

**If you're interested in the expression engine:**
- `crates/fp-expr/src/lib.rs`: start at `Expr`, `parse`, `eval`. Read the top-of-file module docs for the public surface.

**If you're interested in conformance:**
- `crates/fp-conformance/src/lib.rs`: start at `FixtureOperation`, the packet runner, and the comparator. The oracle subprocess machinery lives in the `oracle/` directory.

**If you're interested in window / resample:**
- `crates/fp-frame/src/lib.rs`: search for `impl Rolling`, `impl Expanding`, `impl Ewm`, `impl Resample`. Each has its own reduction implementations.

## Bumpalo Arena Lifetime Mechanics

The arena-backed GroupBy / Join path is performant because *deallocation is free*. Conceptually:

```rust
fn arena_path(input: &DataFrame, by: &[&str], exec: &GroupByExecutionOptions) -> Result<DataFrame, _> {
    let arena = Bump::new();                               // 1 mmap (or none, lazy)
    let estimated_groups = estimate_group_count(input, by);

    // All intermediate structures allocate from the arena, not the global allocator
    let mut buckets: BumpVec<'_, Bucket<'_>> = BumpVec::with_capacity_in(estimated_groups, &arena);
    let mut accumulators: BumpVec<'_, Accumulator<'_>> = BumpVec::with_capacity_in(estimated_groups, &arena);

    // ... per-row processing pushes into bump-allocated buckets ...

    // Materialize the result. The result DataFrame uses the GLOBAL allocator
    // because it outlives the arena.
    let result = materialize_result(&buckets, &accumulators)?;

    // arena drops here. ONE call to munmap (or none) regardless of how many groups we created.
    Ok(result)
}
```

**Lifetimes** keep arena-owned data from leaking into the result. Every `BumpVec<'a, T>` carries the arena's lifetime `'a`, and the compiler refuses to let you store an arena reference past the arena's drop point. The result `DataFrame` is constructed only after each accumulator has been copied (by value) into a heap-allocated `Column`.

**Memory profile**: the arena allocates in slabs starting from `bumpalo`'s `FIRST_ALLOCATION_GOAL` (currently 512 B in `bumpalo` 3.20), doubling on each growth. For a groupby that produces 10K groups with 16 B per accumulator, the arena fits the working set in a small handful of doubled slabs (~256 KB total) and frees all of it in a single drop. Compare to the global path: 10K allocator calls, 10K corresponding `free` calls, fragmentation, fragmentation, fragmentation.

**Falling back to the global path** happens automatically when the arena budget would be exceeded. The budget check is conservative (rejects allocation when the next slab doubling would cross the limit), so very large groupbys cleanly degrade to the unbounded HashMap path rather than panic.

## Comparison Operators and Nullable Boolean Algebra

Pandas comparison operators on nullable Series follow Kleene three-valued logic. FrankenPandas mirrors this exactly. The truth table for `AND`:

| left | right | result |
|------|-------|--------|
| `true` | `true` | `true` |
| `true` | `false` | `false` |
| `false` | `true` | `false` |
| `false` | `false` | `false` |
| `null` | `true` | `null` |
| `null` | `false` | `false` |
| `true` | `null` | `null` |
| `false` | `null` | `false` |
| `null` | `null` | `null` |

`OR` follows the dual:

| left | right | result |
|------|-------|--------|
| `false` | `false` | `false` |
| `true` | `_` | `true` |
| `_` | `true` | `true` |
| `null` | `false` | `null` |
| `false` | `null` | `null` |
| `null` | `null` | `null` |

The implementation in `Series::and` (and `Series::or`) walks both Series element-by-element with explicit `match` arms that order the absorbing cases first:

```rust
// Schematic from fp-frame/src/lib.rs:3541-ish
match (left_scalar, right_scalar) {
    (Scalar::Bool(false), _) | (_, Scalar::Bool(false)) => Scalar::Bool(false),  // absorbing
    (Scalar::Null(k), _)     | (_, Scalar::Null(k))     => Scalar::Null(k),       // propagate
    (Scalar::Bool(a), Scalar::Bool(b))                  => Scalar::Bool(*a && *b),
    _ => /* type error */                                                          ,
}
```

The match ordering is the entire trick: by handling `(false, _)` and `(_, false)` *before* the null-propagation arm, `null AND false` produces `false` instead of `null`. `Series::or` mirrors this with `(true, _)` / `(_, true)` as the absorbing case. The bitwise-on-words approach would be faster but harder to get right; we keep the scalar-match form because every Kleene edge case becomes a single arm and a unit test.

For *comparison* operators (`==`, `!=`, `<`, etc.), nulls always propagate: `null == anything → null`. This drops out of `and_mask(left_valid, right_valid)` being the result validity, regardless of data.

## EvidenceLedger Wire Format

The on-disk JSON-lines representation of a `DecisionRecord` (`crates/fp-runtime/src/lib.rs`):

```json
{
  "ts_unix_ms": 1705314600000,
  "mode": "hardened",
  "action": "reject",
  "issue": {
    "kind": "unknown_feature",
    "subject": "csv_dialect.escape_char",
    "detail": "not in compatibility allowlist"
  },
  "prior_compatible": 0.25,
  "metrics": {
    "posterior_compatible": 0.04,
    "bayes_factor_compatible_over_incompatible": 0.16,
    "expected_loss_allow":  96.0,
    "expected_loss_reject": 0.72,
    "expected_loss_repair": 2.96
  },
  "evidence": [
    {"name": "compatibility_allowlist_miss", "log_likelihood_if_compatible": -3.5, "log_likelihood_if_incompatible": -0.2},
    {"name": "unknown_protocol_field",       "log_likelihood_if_compatible": -2.0, "log_likelihood_if_incompatible": -0.1}
  ]
}
```

Field-by-field:

- `ts_unix_ms`: milliseconds since Unix epoch (`u64`).
- `mode`: `"strict"` or `"hardened"` (`RuntimeMode` enum, snake_case in JSON).
- `action`: `"allow"`, `"reject"`, or `"repair"` (`DecisionAction` enum).
- `issue.kind`: one of `"unknown_feature"`, `"malformed_input"`, `"join_cardinality"`, `"policy_override"` (the four `IssueKind` variants).
- `issue.subject` / `issue.detail`: human-readable subject and detail strings for the issue.
- `prior_compatible`: the prior probability `P(compatible)` (`f64`).
- `metrics`: nested `DecisionMetrics` carrying `posterior_compatible`, `bayes_factor_compatible_over_incompatible`, and three flat `expected_loss_*` fields.
- `evidence`: a `Vec<EvidenceTerm>` where each term has `{name, log_likelihood_if_compatible, log_likelihood_if_incompatible}`. `name` is a `Cow<'static, str>` so the canonical built-in term names (e.g. `compatibility_allowlist_miss`, `unknown_protocol_field`, `estimator_overflow_risk`, `memory_budget_signal`) are stored as static strings without heap allocation.

The ledger is append-only. Operators inspecting historical decisions can replay any entry: given `prior_compatible` + the `evidence` vector + the active `LossMatrix`, the `action` is uniquely determined.

`decision_to_card(record)` (in `fp-runtime`) converts a single ledger entry to a compact, human-readable `GalaxyBrainCard` string for use in CLI output and TUI dashboards.

## Designed-For-Threads, Currently Single-Threaded

The codebase is internally thread-safe (no global mutable state, no `static mut`, no `RefCell` in shared types). `DataFrame`, `Series`, `Column`, `Index`, `MultiIndex`, and `ValidityMask` all implement `Send + Sync` where their components do. `ScalarKey`, `EvidenceLedger`, `DecisionRecord` are all `Send + Sync`.

What we *don't* do today is exploit that thread-safety inside operations. Every `groupby`, `merge`, `rolling` operation runs on the calling thread. Adding rayon-style parallelism is a tracked roadmap item, and the architecture supports it cleanly because:

- Columns are independent. A per-column closure can run on any thread without coordination.
- `ValidityMask` operations are stateless (input → output, no shared state).
- The arena allocator in GroupBy / Join is per-operation, not global; each parallel groupby task could own its own arena.

The current single-threaded baseline still outperforms pandas on most operations because of (a) the AACE alignment-skip fast path, (b) the dense Int64 groupby path, (c) AG-10 typed-array vectorization enabling SIMD auto-vectorization, and (d) zero per-row Python interpretation overhead. Adding parallelism on top of that is a known multiplicative win; it's deferred until other correctness work is done.

## How to Add a New Pandas Method to FrankenPandas

A concrete checklist, based on the patterns the project has converged on:

1. **Identify the method's home crate.** Series and DataFrame methods land in `fp-frame`. Index methods land in `fp-index`. IO formats land in `fp-io`. Aggregations land at the crate root of `fp-types` (the `nan*` helpers are not in a submodule) if they're pure reductions, or in `fp-groupby` if they're group-level.
2. **Look up the pandas signature.** Pin the doc to the same pandas version that `oracle/requirements.txt` pins for the live oracle. Read the pandas source if any edge cases are unclear.
3. **Add a method stub** with the canonical pandas signature (renamed to Rust idiom). Add `Result<_, FrameError>` to anything that can fail at the type-system or domain level.
4. **Write the happy-path implementation.** Use existing helpers wherever possible (`align_union` / `align_inner` / `align_non_unique`, `cast_scalar`, the crate-root `fp_types::nan*` reductions, the typed-array views in `fp-columnar::ColumnData`). Don't reimplement alignment.
5. **Propagate the index / axis name.** This was the focus of the 2026-05 sweep; every helper that produces a new Series/DataFrame should carry the source axis name through unchanged. Look at the existing rolling / expanding / ewm helpers in `fp-frame` to see the canonical pattern.
6. **Add inline `#[cfg(test)]` unit tests** covering happy path, edge cases (empty input, all-null, NaN, infinities), and the pandas-error parity case (which inputs should raise vs return NaN).
7. **Add a conformance packet.** See "Conformance Packet Authoring Guide" above. Both fixture-backed and live-oracle variants. Live-oracle tests live across multiple files under `crates/fp-conformance/src/tests/live_oracle_*.rs` (one file per topic area, e.g. `live_oracle_series_misc.rs`, `live_oracle_dataframe_merge.rs`); add yours to whichever topic matches or create a new file.
8. **Add a property test** if the operation has invariants (commutativity, identity, monotonicity, idempotence). Use `proptest!` macros; seed the corpus with adversarial inputs.
9. **Re-export through the facade.** Add the type / function to `crates/frankenpandas/src/lib.rs` if it should be reachable via `frankenpandas::prelude::*`.
10. **Document any divergence.** If pandas' behavior is surprising or the FrankenPandas implementation differs intentionally, write up a `DISC-NNN` entry in `DISCREPANCIES.md`.
11. **Update the panic contract.** Add a `# Panics` doc comment to the method if it can panic (e.g., on `unwrap`). Document under what conditions. The `cargo doc -D warnings` gate enforces this in CI.
12. **Run `ubs <changed-files>`.** The Ultimate Bug Scanner catches common Rust footguns before commit.
13. **Run `cargo test --workspace`.** Plus `cargo clippy --workspace --all-targets -- -D warnings`.
14. **Close the related Beads issue** with `br close <id>` and `br sync --flush-only`. Commit `.beads/` alongside the code change.

## Why We Forbid `unsafe` Everywhere

Every crate begins with `#![forbid(unsafe_code)]`. Even when there's a measurable performance win available from an `unsafe` block, we don't take it. The rationale:

1. **Multi-agent development**. The project routinely has half a dozen AI agents working in parallel. Code review for `unsafe` correctness is the most fragile review surface; one missed alias, one off-by-one in bounds-elision, and you have memory unsafety that survives all the type-system checks. Forbidding `unsafe` removes the riskiest category of bug from the review surface.
2. **Property tests over `unsafe` are exceptionally hard to write correctly**. The undefined behavior surface of `unsafe` is platform- and optimization-level-dependent, and most "fuzz-tested unsafe" code passes fuzzers but fails when LLVM gets smarter at a later release.
3. **The performance gap is smaller than you'd expect**. AG-10 typed-array vectorization (`&[f64]`, `&[i64]` slice operations) enables SIMD auto-vectorization without `unsafe`. The bumpalo arena gives most of the allocator-win that `unsafe` pointer arithmetic would provide. The expensive paths in pandas (Python interpreter overhead, numpy boundary crossings) don't exist in FrankenPandas regardless.
4. **It survives dependency churn**. The `arrow-rs` crate occasionally introduces a new `unsafe`-required API for performance; we resist using it. The `parquet` crate occasionally renames a `safe` helper; we adapt. Our workspace's `forbid(unsafe_code)` is a permanent invariant, not a per-release re-audit.

The closest we get to `unsafe` is `#[allow(clippy::cast_possible_truncation)]` on intentional `i64 → i32` casts at IO boundaries; annotated rather than silently allowed.

## What's NOT in the Box (Out of Scope, Forever or For Now)

To keep the scope honest:

- **Lazy / streaming execution**. We don't have a query planner; every operation eagerly materializes. Polars and DuckDB are the right tools when you need lazy plans.
- **Out-of-core / spilling**. FrankenPandas operates entirely in memory. For data larger than RAM, write to Parquet/Feather and use DataFusion or DuckDB to stream.
- **Distributed execution**. Single-process. For multi-machine workloads, use Spark / Ray.
- **GPU acceleration**. No CUDA / Vulkan backend. RAPIDS cuDF and Polars-GPU are the right tools.
- **Type-erased dynamic typing**. Every column has a known `DType` at runtime; pandas' "object" dtype maps to `Utf8` plus heterogeneous-payload preservation, not unrestricted `dyn Any`.
- **Pre-built Python bindings** (today). PyO3 bindings are tracked as `br-frankenpandas-4clx`; until they ship, FrankenPandas is Rust-only.
- **A REPL**. No interactive shell. Use a Rust playground or a Jupyter notebook (eventually, via PyO3).
- **A query language other than pandas-style `eval()` / `query()`**. No SQL frontend, no Pythonic chained-method DSL beyond what pandas provides.

## Recommended Workflows by Use Case

**Building a financial backtesting pipeline.**
1. Load trades via `read_csv_str` or `read_parquet_bytes`. Parse timestamps with `to_datetime` + `unit=`.
2. Set the timestamp column as the index via `set_index("timestamp", true)`.
3. Use `merge_asof_with_options` to align trades with quotes / fundamentals / news.
4. `groupby(&["ticker"])?.agg_named(&[...])` for per-ticker aggregations.
5. `rolling(252, Some(20)).std()` for rolling volatility.
6. Export to Feather / Parquet for downstream consumption.

**Building an ML feature pipeline.**
1. Load source data via the appropriate IO format (Parquet for tabular, CSV for raw exports).
2. `convert_dtypes()` or `infer_objects()` to settle dtypes.
3. `fillna(&Scalar::Float64(0.0))` or `dropna()` per column policy.
4. `get_dummies(&["categorical_col"])` for one-hot encoding.
5. `cut(&series, n_bins)` or `qcut(&series, n_quantiles)` for binning.
6. Standard scaling via `df.sub_scalar(mean).div_scalar(std)` per column.
7. Export to Feather (zero-copy interchange with Arrow-compatible ML frameworks).

**Building a streaming ETL job.**
1. Read input chunks via `read_sql_chunks` (SQL) or repeated `read_jsonl_str` calls (JSONL).
2. Apply per-chunk transforms (filter, project, derive new columns).
3. Buffer chunks until reaching a flush threshold.
4. Concat with `concat_dataframes_with_ignore_index` + write via `write_sql` / `write_parquet_bytes`.

**Building a data-quality monitor.**
1. Load reference snapshot + new snapshot.
2. `df1.compare(&df2)?` for element-wise differences.
3. `df1.equals(&df2)` for structural equality boolean.
4. Per-column null counts via `df.count_na()`.
5. Distribution shift detection via `ConformalGuard` from `fp-runtime`.
6. Emit JSON-Lines audit log with the divergent rows.

## A Note on Determinism

FrankenPandas operations are deterministic by default. The only entry points that introduce randomness:

- `DataFrame::sample(n, frac, replace, seed)`: pseudo-random row selection.
- `DataFrame::sample_weights(n, weights, replace, seed)`: weighted sample.

Both accept an `Option<u64>` seed. When `None`, the implementation falls back to a constant default seed (`42`) under the hood; runs without an explicit seed are still reproducible, but they are *identical* across invocations rather than varying with wall-clock time. When `Some(seed)`, output is bit-identical across platforms regardless of OS, Rust version, or CPU. The underlying generator is a Linear Congruential Generator with the well-known PCG/Knuth multiplier `6_364_136_223_846_793_005`.

`HyperLogLog` results are deterministic given the input order. Hash functions used internally are non-cryptographic but stable across versions:

- `ScalarKey` uses `#[derive(Hash)]`, which routes through Rust's `DefaultHasher` (currently SipHash-1-3 in the standard library, subject to whatever the stdlib pins).
- The HyperLogLog register update uses a custom **SplitMix64** finalizer (the canonical `0xbf58_476d_1ce4_e5b9` / `0x94d0_49bb_1331_11eb` multiplier pair) defined in `fp-groupby`.

No operation in FrankenPandas reads from `/dev/urandom`, `getrandom()`, or any OS-level entropy source.

## Glossary (Continued)

| Term | Definition |
|------|-----------|
| **Kleene logic** | Three-valued Boolean algebra (`true` / `false` / `null`) used for nullable Series comparison and combination, matching pandas. `null AND false = false`, `null OR true = true`, every other case with `null` returns `null`. |
| **Identity-alignment fast path (AG-11)** | When both operands of a binary op share an `Index` value (same `Arc`-identity) and the index has no duplicates, the alignment planner is skipped entirely. O(1) check. The single biggest groupby speedup in the codebase. |
| **Borrowed-key HashMap (AG-02)** | The alignment planner builds its position map using `&IndexLabel` references rather than cloning the label. Eliminates O(n) string allocations in the join build phase. |
| **Source-index referencing (AG-08)** | GroupBy's HashMap path stores `(source_row_index, accumulator)` instead of `(key_scalar_clone, accumulator)`. The original `IndexLabel` is reconstructed at output time from the source position, avoiding per-group `Scalar::Utf8` clones. |
| **Compatibility issue** | An ambiguous input encountered at runtime: an unknown protocol field, a join admission outside policy, a malformed input. Each is a `CompatibilityIssue { kind: IssueKind, subject: String, detail: String }` value that gets passed to the Bayesian decision layer. |
| **Decision record** | An entry in the `EvidenceLedger`. Carries the prior, posterior, evidence terms, loss matrix, expected losses, and chosen action. Replayable. |
| **Decision card** | Compact human-readable rendering of a decision record. Produced by `decision_to_card`. Used in CLI output and TUI dashboards. |

## Bibliography of Influences

- **Apache Arrow**, for the bitpacked validity bitmap design (replicated as our `ValidityMask`).
- **Boncz, Zukowski, Nes** — *MonetDB/X100: Hyper-Pipelining Query Execution* (CIDR 2005), for the columnar materialized-execution model.
- **Veldhuizen** — *Leapfrog Triejoin: A Simple, Worst-Case Optimal Join Algorithm* (ICDT 2014), for the N-way join algorithm we adapted to N-way index alignment.
- **Flajolet, Fusy, Gandouet, Meunier** — *HyperLogLog: the analysis of a near-optimal cardinality estimation algorithm* (AOFA 2007), for the approximate-cardinality primitive.
- **Shafer, Vovk** — *A Tutorial on Conformal Prediction* (JMLR 2008), for the calibrated prediction sets used by `ConformalGuard`.
- **Luby, Shokrollahi, Watson, Stockhammer, Minder** — *RaptorQ Forward Error Correction Scheme* (IETF RFC 6330), for the repair-symbol envelope around durable artifacts.
- **McKinney** — *pandas* (2008–present), for the API surface we're matching, and for the precision of the original method semantics that makes a clean-room reimplementation tractable at all.

## Why Forbid `unsafe`: Concrete Examples (Continued)

The four reasons given earlier come up as concrete trade-offs every time we pick up a new optimization opportunity. A representative shortlist:

1. **Bumpalo's safe slot-returning API vs the `unsafe` raw-pointer-bump API.** Bumpalo exposes both. We use the safe one. The raw API would shave a single bounds-check off each push, which the compiler usually elides anyway because the slot count is monomorphic at the call site. Net measured gain: under 1% on the arena-backed groupby microbenchmark.
2. **`get_unchecked` vs `get`.** Replacing `v[i]` with `unsafe { v.get_unchecked(i) }` is the canonical "10× speedup" recipe in C++ porting blog posts. In practice, with Rust's LLVM backend and `#[inline]`, the bounds check is hoisted out of inner loops whenever the compiler can prove the index domain. We've measured the gap on the rolling-sum hot path: about 0.3% in release with `lto=true`, `codegen-units=1`. Not worth the audit surface.
3. **Lifetime-erased Arrow buffer wrappers.** The `arrow-rs` crate's `Buffer` type uses `unsafe` internally to manage refcounted ownership; reaching into its raw bytes from FrankenPandas would require an `unsafe` block on our side. Instead, we always go through the `Buffer::as_slice()` / `Buffer::typed_data()` safe accessors, accept the small reference-count traffic, and stay clean.
4. **`std::ptr::copy_nonoverlapping` for hot-loop column copies.** The `Vec<T>::extend_from_slice` path uses an internal `unsafe` `memcpy` already; calling that from our safe Rust gets the same generated code without the audit surface. We rely on it heavily in the `reindex` paths.

The recurring pattern: when the safe API and the `unsafe` API differ by < 5% measured throughput, we pick safe. When they differ by > 5%, we look for an algorithmic improvement first (better data structure, better cache layout, eliminating the work entirely) before reaching for `unsafe`. So far that algorithmic-first stance has held; no work item has demanded `unsafe` to ship.

## The Eight IO Layers in Detail

Some IO formats have unusual gotchas worth knowing up-front:

**CSV** is the format with the most option surface. The `CsvReadOptions` struct exposes 18 fields including pandas-style `parse_dates`, `usecols`, `dtype`, `na_values`, `keep_default_na`, `quoting`, `comment`, `on_bad_lines`. Reader behavior matches pandas on the major edge cases: embedded newlines in quoted fields, BOM stripping, multi-byte UTF-8, mixed-type column inference (object column becomes `Utf8` when types disagree). The CSV writer canonicalizes Float64 whole numbers to `1.0` to match pandas (rather than `1`); use `dtype` on reads to force them back to `Int64` if you want.

**JSON** supports five orients on read and write: `Records`, `Columns`, `Index`, `Split`, `Values`. `to_json("table")` emits the [JSON Table Schema](https://specs.frictionlessdata.io/table-schema/) format with full Type/Format round-trip. JSONL is also reachable through the `read_jsonl_str` / `write_jsonl_string` standalone functions; the JSONL reader unions all keys across rows, so a "ragged-schema" file (different objects in different rows) becomes a single DataFrame with `null` fill-values for missing keys.

**Parquet** uses Apache Arrow internally; the reader walks `RecordBatch`es and converts each to a Column. Date32, Date64, Timestamp, Time32, Time64, and Decimal128 are all converted to FrankenPandas equivalents (Int64 nanos for temporal types, Float64 with precision flag for decimals). Multi-batch files are streamed and concatenated.

**Excel** uses `calamine` for reads (`.xlsx`, `.xls`, `.xlsb`, `.ods`) and `umya-spreadsheet` for writes. Sheet ordering is preserved (`read_excel_sheets_ordered`). Header detection mirrors pandas. The `to_excel` writer respects index labels and supports the full pandas option matrix (`index_label`, `na_rep`, `merge_cells`, etc.).

**Feather / Arrow IPC stream** are zero-copy interop with anything Arrow-compatible. The file format (Feather v2) has a random-access footer; the stream format is forward-only and used for pipes. We use them as the canonical Rust↔Arrow interchange point.

**SQL** is generic over the `SqlConnection` trait. The bundled implementation is `rusqlite` (gated by the `sql-sqlite` feature). Anyone can implement the trait for their own connection type (PostgreSQL, MySQL, MS SQL) and route the existing `read_sql` / `write_sql` API through it. The `SqlInspector` wrapper gives SQLAlchemy-shaped introspection: `tables`, `views`, `schemas`, `columns`, `indexes`, `foreign_keys`, `unique_constraints`, plus the higher-level `reflect_table` / `reflect_all_tables` / `reflect_all_views`.

**HTML / XML / LaTeX / Markdown** are write-mostly. HTML and XML have readers too (HTML via DOM-style parsing, XML via stream-style). LaTeX and Markdown are write-only; pandas' read paths for these are practically unused in real code.

**Pickle / Stata / ORC / HDF5** are all round-trip-tested but use simpler implementations than pandas:

- **Pickle** uses `serde` + `bincode` under the hood; it's semantically a Rust-canonical binary serde, not a literal pandas-pickle. Round-trip works *within* FrankenPandas; cross-tool interop with Python pandas pickle files is not supported.
- **Stata** supports the common `.dta` formats (114–119); exotic Stata 11 / 12 / 13 features are not implemented.
- **ORC** rides on top of Arrow → `arrow-orc`, with the same dtype-conversion behaviors as Parquet.
- **HDF5** is feature-gated (`hdf5` cargo feature, requires the `hdf5-metno` system dependency). The implementation provides a keyed-snapshot layout: every DataFrame is one HDF5 group with one dataset per column. PyTables-compatible table/storer layouts are a future epic.

## How `eval()` / `query()` Differs From `df["col"] > 5`

The string-based expression engine has three properties Rust closures can't match:

1. **No type annotations needed.** `df.query("price > 100 and volume < 500")` works whether `price` is `Int64` or `Float64`, whether `volume` is `Int64` or `Float64`. Cast promotion happens inside `Expr::evaluate`. A Rust closure-based filter (`df.filter_rows(&mask)`) requires the caller to build the mask with the right dtype-aware comparison.

2. **`@local` lets you parameterize without rebuilding the AST.** `df.query("value > @threshold", &locals)` parses once and binds `@threshold` at evaluation time. A closure equivalent would need to capture the threshold by reference.

3. **Backtick-quoted columns** let you reference column names that aren't valid Rust identifiers (names with spaces, hyphens, leading digits) without escaping. `df.query("`monthly revenue` > 10000")?`. Closure-based code would need a different `column("monthly revenue")` lookup.

The trade-off: parse cost (microseconds), and the engine is interpreted, not JIT-compiled. For one-shot use it's the right call; for a tight inner loop running the same expression over many DataFrames, build a closure once and reuse.

## The `EvidenceLedger` in Practice

A typical hardened-mode run produces a ledger like this:

```
$ tail -3 evidence_ledger.jsonl
{"ts_unix_ms":1715856734001,"mode":"hardened","action":"allow","issue":{"kind":"unknown_feature","subject":"csv_dialect.escape_char","detail":"escape character not in allowlist"},"prior_compatible":0.25,"metrics":{"posterior_compatible":0.78,"bayes_factor_compatible_over_incompatible":9.4,"expected_loss_allow":22.0,"expected_loss_reject":4.68,"expected_loss_repair":2.34},"evidence":[{"name":"compatibility_allowlist_miss","log_likelihood_if_compatible":-3.5,"log_likelihood_if_incompatible":-0.2}]}
{"ts_unix_ms":1715856734102,"mode":"hardened","action":"reject","issue":{"kind":"join_cardinality","subject":"merge(orders, line_items)","detail":"estimated 5.2M rows exceeds cap 1M"},"prior_compatible":0.60,"metrics":{"posterior_compatible":0.04,"bayes_factor_compatible_over_incompatible":0.04,"expected_loss_allow":96.0,"expected_loss_reject":3.6,"expected_loss_repair":7.92},"evidence":[{"name":"estimator_overflow_risk","log_likelihood_if_compatible":-0.3,"log_likelihood_if_incompatible":-1.2}]}
{"ts_unix_ms":1715856734205,"mode":"hardened","action":"repair","issue":{"kind":"malformed_input","subject":"read_csv:row_117_col_3","detail":"value is 'NaN' string, expected Float64"},"prior_compatible":0.25,"metrics":{"posterior_compatible":0.20,"bayes_factor_compatible_over_incompatible":0.78,"expected_loss_allow":80.0,"expected_loss_reject":4.8,"expected_loss_repair":2.40},"evidence":[{"name":"memory_budget_signal","log_likelihood_if_compatible":-1.0,"log_likelihood_if_incompatible":-1.0}]}
```

Reading it: the first entry allowed an unknown CSV dialect option (low prior, but two evidence terms didn't strongly point either way, so the Bayes factor + the loss matrix favored `allow`). The second entry rejected a join because the estimated output cardinality was 5× the policy cap (low posterior, large expected loss of allowing). The third entry repaired a malformed CSV cell by coercing the literal "NaN" string to `Float64(NaN)`; the prior was low, but the loss of rejecting a whole 1M-row CSV over a single bad cell was higher than the loss of repairing in place.

Operators write per-application loss matrices when the defaults aren't right for their domain. A financial firm running a regulatory batch job would set `allow_if_incompatible = 1000.0`, making the runtime almost-always reject; a streaming-analytics shop with continuous backstop validation would use the defaults.

## Pandas Compatibility Status by API Family

A rough heat map of how compatible we are with pandas, by API family, as of 2026-05-16. *Green* = packet-tested, live-oracle-passing, no known DISC entry. *Yellow* = mostly green, but one or more DISC entries note edge cases. *Red* = scaffolded but not yet packet-tested or has multiple open DISC entries.

| Family | Status | Notes |
|--------|--------|-------|
| DataFrame construction | 🟢 | All 12 documented constructors. 15+ entries in the conformance suite. |
| Selection (`loc` / `iloc` / `at` / `iat` / `xs` / `squeeze`) | 🟢 | Including negative-position indexing, boolean masks, regex column filters. |
| Boolean / Kleene logic | 🟢 | All truth-table edge cases match pandas; DISC-005 and DISC-013 are in the Resolved Divergences section. |
| Index alignment (binary ops) | 🟡 | DISC-011/014 note that null introduction doesn't yet promote `Int64` → `Float64` like pandas' nullable extension Int64. |
| Float-zero / NaN groupby keys | 🟢 | Normalized via `ScalarKey`. |
| Window operations (rolling / expanding / ewm / resample) | 🟢 | Validation matches pandas (rejects `min_periods > window`, etc.). |
| GroupBy aggregations (14 reductions + ops) | 🟢 | Including Utf8 lex-sort for `min` / `max` / `idxmin` / `idxmax` / `cummin` / `cummax`. |
| String accessor (50+ methods) | 🟢 | Unicode-correct casefold, char-position (not byte-position) `find` / `rfind`, regex with capture groups. |
| Datetime / Timedelta accessor | 🟢 | Including `to_period` / `to_timestamp` / `isocalendar` / `timetz`. Mixed naive/tz CSV `parse_dates` falls back per DISC-012. |
| Sorting / NA position | 🟢 | Validated `na_position` strings; stable sort. |
| Reshape (`melt`, `pivot_table`, `stack`, `unstack`, `crosstab`, `get_dummies`, `explode`) | 🟢 | Multi-value pivot, custom margins names, with-fill variants. |
| Joins (Inner / Left / Right / Outer / Cross / Asof) | 🟢 | All directions + `tolerance` / `by` / `allow_exact_matches` on asof; `validate=` + `indicator=` + custom `suffixes=` on merge. |
| MultiIndex (row + column) | 🟡 | DISC-006 notes scaffolded-not-full parity for advanced ops. Full parity for set / get / xs / IO round-trip. |
| IO: CSV / JSON / JSONL / Parquet / Excel / Feather / IPC | 🟢 | All seven, including the full pandas option matrices. |
| IO: HTML / XML / LaTeX / Markdown / Pickle / Stata / ORC | 🟢 | All seven, with the caveat that Pickle is FrankenPandas-canonical bincode, not Python-pickle compatible. |
| IO: HDF5 | 🟡 | Feature-gated; keyed-snapshot layout, not PyTables-compatible. |
| IO: SQL (SQLite) | 🟢 | Full read / write / chunked / inspector surface. |
| IO: SQL (PostgreSQL / MySQL / others) | 🔴 | Generic trait is in place; bundled adapters are not. Tracked under `br-frankenpandas-fd90`. |
| Sparse (`.sparse()` accessor + `SparseDType`) | 🟡 | DISC-009: accessor surface works but physical storage is still dense. |
| `apply` shape variants | 🟡 | DISC-010: Rust requires explicit shape (`apply_scalar` / `apply_series` / `apply_series_stacked`). Function-wise equivalent. |
| Python bindings (PyO3) | 🔴 | Not shipped. Tracked under `br-frankenpandas-4clx` release umbrella. |
| Plotting (`plot` / `hist` / `boxplot`) | 🟡 | Returns backend-neutral `PlotSpec` / `BoxPlotSpec` / `HistogramSpec` data. Renderer is deferred. |
| Clipboard / GBQ | 🔴 | Deferred. |

## Worked Example: Multi-Source Time-Series ETL

A complete pipeline that touches most of the API surface:

```rust
use frankenpandas::prelude::*;
use std::path::Path;
use std::collections::BTreeMap;

// 1. Load trades from Parquet and quotes from a SQL database.
let trades = read_parquet_bytes(&std::fs::read("trades.parquet")?)?;
let quotes_conn = frankenpandas::rusqlite::Connection::open("quotes.db")?;
let quotes = read_sql_with_options(
    &quotes_conn,
    "SELECT timestamp, ticker, bid, ask FROM quotes WHERE date = '2024-01-15'",
    &SqlReadOptions::default(),
)?;

// 2. Set timestamp as the index on both sides.
let trades = trades.set_index("timestamp", /* drop */ true)?;
let quotes = quotes.set_index("timestamp", /* drop */ true)?;

// 3. Asof-merge: each trade gets the most recent quote within 5 seconds,
//    matched on ticker (equi-join key) before the time-asof step.
let merged = merge_asof_with_options(
    &trades, &quotes, "timestamp", AsofDirection::Backward,
    MergeAsofOptions {
        tolerance: Some(5_000_000_000.0),                // 5 seconds, expressed in nanos
        by: Some(vec!["ticker".to_owned()]),
        allow_exact_matches: true,
    },
)?;
let enriched = DataFrame::new(merged.index, merged.columns)?;

// 4. Compute mid-price and effective spread.
let with_mid = enriched.assign_fn(vec![
    ("mid", Box::new(|df| {
        let bid = df.column("bid").expect("bid column");
        let ask = df.column("ask").expect("ask column");
        let values: Vec<Scalar> = bid.values().iter().zip(ask.values()).map(|(b, a)| {
            match (b, a) {
                (Scalar::Float64(bv), Scalar::Float64(av)) => Scalar::Float64((bv + av) / 2.0),
                _ => Scalar::Null(NullKind::NaN),
            }
        }).collect();
        Column::from_values(values).map_err(FrameError::from)
    })),
])?;

// 5. Per-ticker rolling volatility (30-tick standard deviation of mid-price).
let by_ticker = with_mid.groupby(&["ticker"])?;
let vol_30 = by_ticker.rolling(30, None)?.std()?;

// 6. Aggregate to a daily summary.
let daily = with_mid.groupby(&["ticker"])?.agg_named(&[
    ("trade_count",  "ticker", "count"),
    ("avg_price",    "mid",    "mean"),
    ("price_std",    "mid",    "std"),
    ("max_spread",   "spread", "max"),       // assuming a "spread" column was added similarly
])?;

// 7. Export the rolling volatility as Feather (for the next pipeline stage),
//    and the daily summary as HTML (for the dashboard).
write_feather_bytes(&vol_30)?;
let dashboard_html = daily.to_html(true)?;
std::fs::write("daily_summary.html", dashboard_html)?;
```

Lines exercised: 8 IO formats, 1 SQL inspector, AACE index alignment (set_index → merge_asof → groupby), the assign_fn closure form, the named-aggregation triple form, rolling window with a min_periods None default, the `EvidenceLedger` (every alignment in there gets a ledger entry under hardened mode).

## The Project's Five Bedrock Invariants

Some things the codebase will never compromise on. If a proposed change would violate one of these, it gets rejected during review:

1. **No `unsafe` code anywhere.** Workspace-wide `#![forbid(unsafe_code)]`. Discussed in detail above.
2. **Differential conformance is non-negotiable.** Every operation has a pandas-oracle-pinned reference. We don't ship behavior that disagrees with pandas unless that disagreement is in `DISCREPANCIES.md` with full root-cause analysis and explicit status (ACCEPTED / INVESTIGATING / WILL-FIX).
3. **Three-way null semantics**. `Null`, `NaN`, `NaT` are distinct kinds that map to pandas exactly. The validity mask is the single source of truth for "is this missing"; the data slot is undefined when validity is 0.
4. **Zero panics in public APIs.** Every public function that can fail returns `Result<_, _>`. Internal `unwrap` calls are gated by debug-mode assertions or annotated with `# Panics` doc comments. The `cargo doc -D warnings` CI gate enforces the documentation.
5. **Determinism by default**. Same inputs → same outputs, every time, across platforms. The only randomized entry points (`sample`, `sample_weights`) take a `seed: Option<u64>` and use a deterministic LCG.

## What `fp-runtime` Does That `fp-frame` Can't

A natural question: why is there a separate `fp-runtime` crate at all? The split exists because `fp-runtime` adds capabilities orthogonal to data operations:

- **`RuntimePolicy`** decides how the runtime should react to ambiguous inputs. Strict mode fails fast; hardened mode runs the Bayesian decision layer. Either way, `fp-frame` operations call into `fp-runtime` when they encounter a `CompatibilityIssue`.
- **`ConformalGuard`** wraps any binary classification ("is this batch in-distribution?") with a calibrated confidence set using split-conformal inference. The classification problem is your own; `ConformalGuard` produces a *threshold* that's been calibrated on a held-out dataset, plus a confidence parameter `α`, plus a prediction set per new input. It's used by the conformance gate to detect distribution shift in input fixtures: if a packet's input distribution starts diverging from the baseline, the guard flips red even if the parity comparison would have passed.
- **`EvidenceLedger`** is the append-only journal of every policy-driven decision. Independent from data operations; runs in parallel.
- **`RaptorQ envelope`** wraps durable artifacts (conformance fixtures, benchmark baselines, migration manifests) with Forward Error Correction repair symbols. The envelope includes a manifest, integrity-scrub report, and decode proof per recovery event. Detects bit-rot before it's an outage.

`fp-frame` could in principle embed all of this directly. The split is for two reasons: (a) you can `default-features = false` to drop `fp-runtime` entirely if you want a leaner DataFrame-only build, and (b) `fp-runtime` is the natural home for additions that aren't strictly per-operation (rate limiting, cross-cutting policy, telemetry).

## Comparison With Polars and DuckDB (Honest Version)

Both Polars and DuckDB are excellent and overlap with FrankenPandas on parts of the surface. Where each shines:

**Polars** is the right tool if you:
- Want lazy / streaming execution and a real query planner (Polars optimizes across chains; FrankenPandas doesn't).
- Have data larger than RAM and need out-of-core processing.
- Are comfortable rewriting pandas idioms into Polars' expression DSL (it's a clean DSL but a different API).
- Want production-grade parallelism today (Polars uses rayon by default; FrankenPandas is single-threaded).

**DuckDB** is the right tool if you:
- Want to express the analysis in SQL.
- Need fast aggregations over multi-GB Parquet files without loading them into memory.
- Want Arrow-native zero-copy interop and a mature, well-exercised query optimizer.
- Don't need pandas-shaped DataFrame semantics; DuckDB returns columnar query results, not DataFrames with index alignment.

**FrankenPandas** is the right tool if you:
- Want pandas semantics specifically: every dtype promotion, NaN propagation, index-alignment quirk, edge-case ordering rule.
- Are writing Rust and don't want to learn a new DSL or wire up a Python interpreter.
- Care about explicit, auditable alignment decisions (AACE + EvidenceLedger).
- Need the full pandas API surface, not the most-used 80% subset.
- Don't currently need out-of-core or distributed execution; the working set fits in RAM.

The three projects compose well: use DuckDB / Polars for the heavy filter/aggregate pass, hand the result to FrankenPandas (via Feather / Parquet) for the pandas-shaped transformations downstream, then export back.

## About Contributions

Please don't take this the wrong way, but I do not accept outside contributions for any of my projects. I simply don't have the mental bandwidth to review anything, and it's my name on the thing, so I'm responsible for any problems it causes; thus, the risk-reward is highly asymmetric from my perspective. I'd also have to worry about other "stakeholders," which seems unwise for tools I mostly make for myself for free. Feel free to submit issues, and even PRs if you want to illustrate a proposed fix, but know I won't merge them directly. Instead, I'll have Claude or Codex review submissions via `gh` and independently decide whether and how to address them. Bug reports in particular are welcome. Sorry if this offends, but I want to avoid wasted time and hurt feelings. I understand this isn't in sync with the prevailing open-source ethos that seeks community contributions, but it's the only way I can move at this velocity and keep my sanity.

## License

MIT License (with OpenAI/Anthropic Rider). See `LICENSE`.
