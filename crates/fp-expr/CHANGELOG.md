# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/fp-expr-v0.1.0) - 2026-04-26

### <!-- 0 -->Added

- *(io)* gate sqlite sql backend (br-frankenpandas-fd90)
- *(fp-frame)* land row_multiindex DataFrame field (br-frankenpandas-1zzp.1)
- *(fp-frame)* add to_markdown tablefmt options (frankenpandas-ot0k)
- *(frame)* implement DataFrame.melt with mixed-type numeric promotion
- *(fp-frame)* rank method/na_option validation, mode edge cases, merge_asof + rank conformance
- add mod/pow/floordiv operators, row-wise apply, and extensive conformance fixes
- *(expr,frame)* add @local variable bindings to expression engine and expand DataFrame operations
- *(conformance)* add --python-bin CLI flag and join benchmark binary
- add Series convert_dtypes/map_with_na_action/case_when, Index utility methods, and DataFrame extension traits
- major DataFrame frame implementation rewrite and expression engine
- *(phase2c)* expand compat-closure evidence packs FP-P2C-006 through FP-P2C-011
- *(workspace)* add fp-frankentui crate scaffold and update workspace dependencies
- complete essence extraction for FP-P2C-006..011 and expand columnar/frame/groupby implementations
- *(expr)* add incremental view maintenance with delta propagation

### <!-- 1 -->Fixed

- *(api)* gate all pub error enums with #[non_exhaustive] (br-frankenpandas-tne4)
- *(docs)* fix misaligned right-side borders in all 3 ASCII art diagrams

### <!-- 4 -->Documentation

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
- *(fp-expr)* add crate-level rustdoc (br-el32 / fd90.66)
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

- *(fp-expr)* add eval/query expression parity tests (frankenpandas-xmp)

### Fp-expr

- add DataFrame-backed evaluation bridge via EvalContext (bd-3l4a)
- add comparison expression operators and delta evaluation (bd-1mf7)
- port Sub/Mul/Div expression operators (bd-1wfi)

### Fp-expr/fp-frame

- add boolean mask composition operators and planner logical ops (bd-9ksz)

### Licensing

- adopt MIT + OpenAI/Anthropic rider across workspace and crates

### Style

- normalize rustfmt formatting across fp-expr, fp-frame, fp-groupby, fp-io, fp-join
