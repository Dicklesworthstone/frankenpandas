# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/frankenpandas-v0.1.0) - 2026-04-25

### <!-- 0 -->Added

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

- *(docs)* fix misaligned right-side borders in all 3 ASCII art diagrams
- *(frankenpandas)* add error types and NullKind/Column to facade API

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

### Licensing

- adopt MIT + OpenAI/Anthropic rider across workspace and crates
