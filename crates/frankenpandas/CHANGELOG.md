# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/frankenpandas-v0.1.0) - 2026-04-26

### <!-- 0 -->Added

- *(fp-io)* SqlInspector::reflect_table for bundled table metadata (br-76mw / fd90.40)
- *(fp-io)* SqlInspector wrapper for unified introspection API (br-szs9 / fd90.38)
- *(fp-io)* SqlConnection::table_comment for backend-agnostic comment introspection (br-yu3w / fd90.32)
- *(fp-io)* SqlConnection::list_unique_constraints + tighten list_indexes (br-sh4v / fd90.31)
- *(fp-io)* SqlConnection::list_views for backend-agnostic view discovery (br-gm3r / fd90.30)
- *(fp-io)* SqlConnection::list_foreign_keys for backend-agnostic FK introspection (br-uht8 / fd90.29)
- *(fp-io)* SqlConnection::list_indexes for backend-agnostic index introspection (br-bgv9 / fd90.28)
- *(fp-io)* SqlConnection::max_identifier_length capability probe (br-cs81 / fd90.26)
- *(fp-io)* SqlConnection::primary_key_columns derived helper (br-uw3y / fd90.25)
- *(fp-io)* SqlConnection::server_version for backend version probing (br-e23k / fd90.24)
- *(fp-io)* SqlConnection::truncate_table for fast table reset (br-phum / fd90.23)
- *(fp-io)* SqlConnection::list_schemas for backend-agnostic schema discovery (br-lxhi / fd90.22)
- *(fp-io)* SqlConnection::table_schema for backend-agnostic column introspection (br-w43q / fd90.21)
- *(fp-io)* SqlConnection::list_tables for backend-agnostic table discovery (br-vhq2 / fd90.20)
- *(fp-io)* add SQL table option index reads (frankenpandas-fd90.8)
- *(fp-io)* add SQL table read options (frankenpandas-fd90.7)
- *(fp-io)* add SQL table projection index chunks (frankenpandas-fd90.6)
- *(fp-io)* add optioned indexed SQL chunks (frankenpandas-fd90.5)
- *(fp-io)* add indexed SQL chunk reads (frankenpandas-fd90.4)
- *(fp-io)* add read_sql_table column chunks (frankenpandas-fd90.3)
- *(fp-io)* add read_sql_table chunks (frankenpandas-fd90.2)
- *(fp-io)* add read_sql_query chunk aliases (frankenpandas-fd90.1)
- *(fp-io)* add SQL chunked reads (frankenpandas-1e2i)
- *(io)* gate sqlite sql backend (br-frankenpandas-fd90)
- *(sql)* add generic SQL connection foundation (br-frankenpandas-fd90)
- *(fp-frame)* land row_multiindex DataFrame field (br-frankenpandas-1zzp.1)
- add Series.reset_index DataFrame parity (br-52lj)
- *(fp-io)* add SQL write index parity (frankenpandas-t7wn)
- *(fp-io)* add SQL parse_dates parity (frankenpandas-xfrv)
- *(fp-frame)* add to_markdown tablefmt options (frankenpandas-ot0k)
- add top-level frankenpandas facade crate with unified public API (frankenpandas-nsf)
- *(conformance)* add --python-bin CLI flag and join benchmark binary
- *(phase2c)* expand compat-closure evidence packs FP-P2C-006 through FP-P2C-011
- *(workspace)* add fp-frankentui crate scaffold and update workspace dependencies

### <!-- 1 -->Fixed

- *(frankenpandas)* forward tracing + asupersync features (br-ii9u / fd90.59)
- *(frankenpandas)* forward fp-io sql-* features to umbrella (br-l6nr / fd90.58)
- *(docs)* fix misaligned right-side borders in all 3 ASCII art diagrams
- *(frankenpandas)* add error types and NullKind/Column to facade API

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
- *(workspace)* enable rustdoc::broken_intra_doc_links on all crates (br-ddy4 / fd90.71)
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

### Licensing

- adopt MIT + OpenAI/Anthropic rider across workspace and crates
