# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/fp-io-v0.1.0) - 2026-04-25

### <!-- 0 -->Added

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

### <!-- 4 -->Documentation

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
