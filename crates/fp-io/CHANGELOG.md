# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/fp-io-v0.1.0) - 2026-04-26

### <!-- 0 -->Added

- *(fp-io)* SqlInspector::reflect_all_views (br-zuqt / fd90.54)
- *(fp-io)* SqlInspector::reflect_all_tables for whole-database introspection (br-jmmo / fd90.53)
- *(fp-io)* SqlReflectedTable per-column index/UC accessors (br-37uy / fd90.52)
- *(fp-io)* SqlReflectedTable accessor methods (br-63ac / fd90.51)
- *(fp-io)* SqlInspector::reflect_table for bundled table metadata (br-76mw / fd90.40)
- *(fp-io)* SqlInspector::has_column / column ergonomic helpers (br-ppry / fd90.39)
- *(fp-io)* SqlInspector wrapper for unified introspection API (br-szs9 / fd90.38)
- *(fp-io)* SqlColumnSchema::autoincrement field for autoincrement metadata (br-bkl2 / fd90.37)
- *(fp-io)* SqlReadOptions::index_col for unified index_col on read paths (br-c1h9 / fd90.36)
- *(fp-io)* SqlColumnSchema::comment field for column-level comments (br-cfld / fd90.35)
- *(fp-io)* SqlReadOptions::columns for unified projection-during-read (br-d3e9 / fd90.34)
- *(fp-io)* SqlWriteOptions::chunksize for transaction-size-bounded to_sql (br-ls9z / fd90.33)
- *(fp-io)* SqlConnection::table_comment for backend-agnostic comment introspection (br-yu3w / fd90.32)
- *(fp-io)* SqlConnection::list_unique_constraints + tighten list_indexes (br-sh4v / fd90.31)
- *(fp-io)* SqlConnection::list_views for backend-agnostic view discovery (br-gm3r / fd90.30)
- *(fp-io)* SqlConnection::list_foreign_keys for backend-agnostic FK introspection (br-uht8 / fd90.29)
- *(fp-io)* SqlConnection::list_indexes for backend-agnostic index introspection (br-bgv9 / fd90.28)
- *(fp-io)* write_sql identifier-length validation (br-9ynk / fd90.27)
- *(fp-io)* SqlConnection::max_identifier_length capability probe (br-cs81 / fd90.26)
- *(fp-io)* SqlConnection::primary_key_columns derived helper (br-uw3y / fd90.25)
- *(fp-io)* SqlConnection::server_version for backend version probing (br-e23k / fd90.24)
- *(fp-io)* SqlConnection::truncate_table for fast table reset (br-phum / fd90.23)
- *(fp-io)* SqlConnection::list_schemas for backend-agnostic schema discovery (br-lxhi / fd90.22)
- *(fp-io)* SqlConnection::table_schema for backend-agnostic column introspection (br-w43q / fd90.21)
- *(fp-io)* SqlConnection::list_tables for backend-agnostic table discovery (br-vhq2 / fd90.20)
- *(fp-io)* SqlWriteOptions::method for multi-row INSERT batching (br-i0ml / fd90.19)
- *(fp-io)* SqlWriteOptions::dtype per-column SQL-type override (br-ev2s / fd90.18)
- *(fp-io)* table_exists_in_schema for schema-aware existence checks (br-70d1 / fd90.17)
- *(fp-io)* schema-qualified DROP TABLE in write_sql Replace branch (br-hxob / fd90.16)
- *(fp-io)* SqlWriteOptions::schema for cross-schema writes (br-udn6 / fd90.15)
- *(fp-io)* SqlReadOptions::schema for cross-schema reads (br-u6zn / fd90.14)
- *(fp-io)* SqlConnection schema-introspection probes (br-6dk9 / fd90.13)
- *(fp-io)* SqlReadOptions::dtype per-column override (br-l9pt / fd90.11)
- *(fp-io)* SqlConnection::quote_identifier overridable (br-2y7w / fd90.10)
- *(fp-io)* SqlConnection capability + dialect probes (br-frankenpandas-6dtf, fd90.9)
- *(fp-io)* add SQL table option index reads (frankenpandas-fd90.8)
- *(fp-io)* add SQL table read options (frankenpandas-fd90.7)
- *(fp-io)* add SQL table projection index chunks (frankenpandas-fd90.6)
- *(fp-io)* add optioned indexed SQL chunks (frankenpandas-fd90.5)
- *(fp-io)* add indexed SQL chunk reads (frankenpandas-fd90.4)
- *(fp-io)* add read_sql_table column chunks (frankenpandas-fd90.3)
- *(fp-io)* add read_sql_table chunks (frankenpandas-fd90.2)
- *(fp-io)* add read_sql_query chunk aliases (frankenpandas-fd90.1)
- *(fp-io)* add SQL chunked reads (frankenpandas-1e2i)
- *(fp-io)* add SQL coerce_float reads (frankenpandas-zadn)
- *(io)* gate sqlite sql backend (br-frankenpandas-fd90)
- *(sql)* add generic SQL connection foundation (br-frankenpandas-fd90)
- *(io)* round-trip row multiindex across formats (br-frankenpandas-1zzp.4)
- *(fp-frame)* land row_multiindex DataFrame field (br-frankenpandas-1zzp.1)
- *(fp-io)* add read_sql params parity (br-frankenpandas-tk3k)
- *(fp-io)* add read_csv parse_dates dict-style rename parity (br-frankenpandas-cxtw)
- *(fp-io)* add SQL write index parity (frankenpandas-t7wn)
- *(fp-io)* add read_sql_table columns subset parity (frankenpandas-c2zc)
- *(fp-io)* add SQL parse_dates parity (frankenpandas-xfrv)
- *(fp-io)* add read_sql index_col promotion parity (frankenpandas-1v8y)
- *(fp-io)* add to_excel sheet_name/index/index_label/header parity (frankenpandas-emw9)
- *(fp-io)* add read_excel_sheets_ordered preserving workbook order (frankenpandas-wrt3)
- *(fp-io)* add named headerless Excel read parity (frankenpandas-5tz7)
- *(fp-io)* add csv index_label parity (frankenpandas-c1dk)
- *(fp-io)* add read_csv lineterminator parity (frankenpandas-lb88)
- *(fp-io)* add read_csv skipfooter parameter parity (frankenpandas-jy6q)
- *(fp-io)* add write_csv options struct parity (frankenpandas-djye)
- *(fp-io)* add read_csv quote/escape option parity (frankenpandas-oukm)
- *(fp-io)* add read_csv thousands separator parity (frankenpandas-xo9e)
- *(fp-io)* add combined parse_dates parity (frankenpandas-b8ov)
- *(fp-frame)* report categorical series dtype (frankenpandas-fq5u)
- *(io)* CSV parse_dates support with mixed-timezone strict fixture (FP-P2D-429)
- *(io)* CSV true_values / false_values parity with pandas (FP-P2D-426)
- *(fp-io)* add read_csv decimal separator parity (frankenpandas-b7yx)
- *(fp-io)* add read_csv on_bad_lines parity (frankenpandas-rzbu)
- *(fp-io)* add read_csv comment parameter parity (frankenpandas-s6vq)
- *(fp-io)* add series arrow nullable-int roundtrip (frankenpandas-huin)
- *(fp-frame)* add to_markdown tablefmt options (frankenpandas-ot0k)
- *(io)* Excel roundtrip with index label preservation
- *(conformance)* IO round-trip fixture ops + reshape/pivot oracle coverage
- *(frame)* implement DataFrame.melt with mixed-type numeric promotion
- *(fp-io)* add pandas-default NA value handling options
- add mod/pow/floordiv operators, row-wise apply, and extensive conformance fixes
- *(fp-io)* add JSONL (JSON Lines) read/write I/O (frankenpandas-sue)
- *(fp-io)* expand CsvReadOptions with usecols, nrows, skiprows, dtype (frankenpandas-qoz)
- *(io)* add usecols, nrows, skiprows, and dtype options to CSV reader
- *(fp-io)* add Arrow IPC / Feather v2 read/write I/O (frankenpandas-98n)
- *(io)* add Arrow IPC (Feather v2) read/write support
- *(fp-io)* add SQL (SQLite) I/O support via rusqlite (frankenpandas-5ha)
- *(fp-io,fp-conformance)* add Excel I/O and property-based fuzz tests (frankenpandas-6s1, frankenpandas-x2n)
- *(conformance)* add --python-bin CLI flag and join benchmark binary
- *(io)* support headerless CSV input with auto-generated column names
- add Series convert_dtypes/map_with_na_action/case_when, Index utility methods, and DataFrame extension traits
- *(fp-io)* add Parquet read/write support via Arrow RecordBatch integration
- *(fp-frame)* implement 15 new Series methods, 4 new DataFrame methods, and fix pandas conformance in groupby/types
- *(io)* add MissingIndexColumn error variant and CSV edge-case tests
- *(phase2c)* expand compat-closure evidence packs FP-P2C-006 through FP-P2C-011
- *(workspace)* add fp-frankentui crate scaffold and update workspace dependencies
- complete essence extraction for FP-P2C-006..011 and expand columnar/frame/groupby implementations
- *(io)* add JSON IO, CSV options, and file-based read/write
- expand groupby aggregation, join operations, and conformance testing
- *(conformance)* expand differential conformance harness with pandas parity testing

### <!-- 1 -->Fixed

- *(fp-io)* schema validation error wording (br-597l / fd90.56)
- *(fp-io)* escape underscore in sqlite_master LIKE filter (br-s69b / fd90.50)
- *(fp-io)* cfg-gate make_sql_test_conn tests for clean --no-default-features (br-7a49 / fd90.48)
- *(fp-io)* gate SQL ident helpers on sql-sqlite feature (br-ld8h / fd90.45)
- *(fp-io)* list_foreign_keys resolves implicit-PK references (br-cs1r / fd90.44)
- *(fp-io)* reflect_table avoids redundant table_schema round-trip (br-2kzv / fd90.43)
- *(fp-io)* tighten autoincrement detection for composite PKs (br-3i43 / fd90.42)
- *(fp-io)* SqlReadOptions::coerce_float default = true matches pandas (br-o0x6 / fd90.41)
- *(fp-io)* match pandas JSONL and Parquet edge parity
- *(dtype)* wire sparse marker through crate policies (frankenpandas-0xcm)
- *(io)* align csv and json parity
- *(datetime)* coerce mixed timezone to_datetime
- *(api)* gate all pub error enums with #[non_exhaustive] (br-frankenpandas-tne4)
- *(fp-io)* restore excel default round-trip (br-frankenpandas-ho6t)
- *(io)* reject duplicate column names in readers (frankenpandas-akb)
- *(docs)* fix misaligned right-side borders in all 3 ASCII art diagrams
- *(fp-io)* JSONL reader now unions all keys across rows
- *(fp-io)* rename misleading adversarial test name
- *(fp-io)* reject empty SQL table names in validation
- *(fp-io,fp-conformance)* fix 3 bugs found in code review
- *(fp-io)* align JSON orients with pandas (bd-q2ui)

### <!-- 3 -->Changed

- *(fp-io)* dry up identifier validators (br-4l7a / fd90.55)
- *(fp-io)* split SQL test imports surgically (br-m0sw / fd90.49)
- *(fp-io)* dry up primary_key_columns via primary_keys_from_schema (br-oghl / fd90.47)
- *(fp-io)* thread quote_identifier trait dispatch through SQL helpers (br-cx2x / fd90.12)

### <!-- 4 -->Documentation

- *(readme)* replace non-existent Series::constant with from_values pattern (br-5vg7r / fd90.111)
- *(readme)* fix scores.ge(&Scalar) — should be ge_scalar (br-jxwwk / fd90.110)
- *(readme)* add missing inclusive arg to series.between() example (br-luqdy / fd90.109)
- *(readme)* add missing name arg to apply_row example (br-0j4xa / fd90.108)
- *(readme)* fix df.transform() example to match real signatures (br-7135r / fd90.107)
- *(readme)* correct squeeze return-type chain (br-szafc / fd90.106)
- *(readme)* fix df.loc(str) + df.loc_rows() example to match real signature (br-cco1r / fd90.105)
- *(readme)* fix non-existent dropna_with_thresh API name (br-dsnz4 / fd90.104)
- *(readme)* expand NanOps section to actual 19+4 primitives (br-0xxdr / fd90.103)
- *(readme)* correct DataFrameIoExt format coverage 'all'→'7 of 8' (br-rm2mv / fd90.102)
- *(readme)* correct MissingIndexColumn arity in IoError row (br-f7sjm / fd90.101)
- *(readme)* reconcile architecture diagram with '12 crates' headline (br-i8gsh / fd90.100)
- *(readme)* correct AG-13 attribution + lookup path for unsorted (br-i9rhv / fd90.99)
- *(readme)* fix 5x OnceCell→OnceLock terminology mismatch (br-uvb40 / fd90.98)
- *(readme)* extend GroupBy decision tree to show all 3 paths (br-qmthc / fd90.97)
- *(readme)* add row_multiindex + column_multiindex to DataFrame ASCII tree (br-gnzcw / fd90.96)
- *(readme)* correct Series pseudo-code to match real struct (br-u73po / fd90.95)
- *(readme)* correct DType::Categorical existence claim (br-agabe / fd90.94)
- *(readme)* fix optimization technique count 14→15 (br-1teu1 / fd90.93)
- *(readme)* sync crate-tree LOC + pub fn counts (br-ps8bh / fd90.92)
- *(readme)* replace stale 'all green' conformance claim w/ honest disclosure (br-jow1v / fd90.91)
- *(readme)* sync test count badge 1500+ → 3200+ (br-5zni / fd90.90)
- *(readme)* sync 7→8 IO format count for Arrow IPC stream (br-ym7n / fd90.89)
- *(readme)* sync Data Model enum lists with current fp-types/fp-index (br-2chb)
- *(README)* add Round 1 to performance optimization table (br-zxh3 / fd90.87)
- *(README)* update adversarial test count + bullet coverage (br-sjvn / fd90.86)
- *(README)* sync FAQ test count to 3,200+ (br-1nnd / fd90.85)
- *(README)* add Cargo features section reflecting umbrella forwarding (br-2wb6 / fd90.84)
- *(README)* clarify DataFrame output method count (br-qs9u / fd90.83)
- *(README)* link to DISCREPANCIES.md for known-failure root causes (br-w69a / fd90.82)
- *(README)* correct method count + soften 'all green' claim (br-wgth / fd90.75)
- *(README)* refresh test count table (br-d565 / fd90.74)
- *(README)* correct SQL feature claims (br-iew1 / fd90.73)
- *(fp-io)* wrap bare URL in doc comment (br-6sus / fd90.72)
- *(workspace)* enable rustdoc::broken_intra_doc_links on all crates (br-ddy4 / fd90.71)
- *(fp-io)* add crate-level rustdoc summarizing scope (br-omrv / fd90.60)
- *(fp-io)* correct default_schema() fallback claims (br-bn7x / fd90.57)
- surface Arrow IPC stream as 8th IO format (br-frankenpandas-zbz5)
- per-crate README.md for each fp-* crate (br-frankenpandas-kw5q)
- close row MultiIndex epic (br-frankenpandas-1zzp)
- align conformance claim to actual 430+ packets / 1249 fixtures (br-frankenpandas-zgqj)
- scope SQL parity claim to SQLite-only (br-frankenpandas-m3e8)
- link row MultiIndex roadmap to epic (br-frankenpandas-0yz7)
- split MultiIndex capability into Row vs Column (br-frankenpandas-0yz7)
- qualify drop-in positioning until PyO3 ships (br-frankenpandas-diic)
- rustfmt pass and new API surface across fp-index, fp-io, fp-join, fp-frame, fp-expr, and frankenpandas
- expand README to 1574 lines, de-slopify all AI writing artifacts
- expand README to 1440+ lines with missing data, coercion, element-wise ops, selection, introspection
- expand README to 1200+ lines with optimization catalog, error architecture, constructors, merge options
- expand README to 1000+ lines with recipes, deep dives, roadmap
- expand README with deep technical content (+300 lines)
- complete README.md rewrite reflecting current project state
- update README with project status and architecture overview

### <!-- 5 -->Testing

- *(fp-io)* add 15 adversarial parser tests for CSV/JSON/SQL (frankenpandas-yby)

### Fp-io

- preserve split-orient JSON index labels on roundtrip (bd-3aur)
- add file-path CSV options reader (bd-3d8d)
- add JSON orient=index read/write path (bd-15ly)

### Licensing

- adopt MIT + OpenAI/Anthropic rider across workspace and crates

### Style

- *(fp-columnar,fp-frame,fp-io)* clippy and rustfmt cleanup
- normalize rustfmt formatting across fp-expr, fp-frame, fp-groupby, fp-io, fp-join
