# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/fp-types-v0.1.0) - 2026-04-26

### <!-- 0 -->Added

- *(fp-types)* period_range builder (br-2jef / epoj Phase 2)
- *(fp-types)* Timestamp string-unit rounding + public unit_to_nanos (br-lbsx)
- *(fp-types)* Timestamp floor_to/ceil_to/round_to (br-5h6n / 9p0u Phase 2.5)
- *(fp-types)* Timestamp struct + Timedelta arithmetic (br-9p0u / 4r56 Phase 2)
- *(fp-types)* Timedelta arithmetic ops (br-frankenpandas-4r56 Phase 1)
- *(fp-types)* interval_range builders (br-frankenpandas-xaom)
- *(fp-types)* Period + PeriodFreq scaffolding (br-frankenpandas-epoj)
- *(fp-types)* Interval + IntervalClosed scaffolding (br-frankenpandas-j8k4)
- *(io)* gate sqlite sql backend (br-frankenpandas-fd90)
- *(fp-frame)* land row_multiindex DataFrame field (br-frankenpandas-1zzp.1)
- *(fp-types)* add nan moment parity (frankenpandas-vob5 frankenpandas-9bhj frankenpandas-poc0 frankenpandas-bxcm)
- *(fp-types)* add nancumsum/nancumprod/nancummax/nancummin/nanquantile/nanarg parity (frankenpandas-hexr)
- *(fp-frame)* report categorical series dtype (frankenpandas-fq5u)
- *(fp-frame)* add to_markdown tablefmt options (frankenpandas-ot0k)
- add any and all for DataFrameGroupBy
- *(frame)* implement DataFrame.melt with mixed-type numeric promotion
- *(fp-frame)* rank method/na_option validation, mode edge cases, merge_asof + rank conformance
- add mod/pow/floordiv operators, row-wise apply, and extensive conformance fixes
- *(conformance)* add --python-bin CLI flag and join benchmark binary
- *(groupby,types)* add nunique, prod, and size aggregation functions
- *(fp-frame)* implement 15 new Series methods, 4 new DataFrame methods, and fix pandas conformance in groupby/types
- *(phase2c)* expand compat-closure evidence packs FP-P2C-006 through FP-P2C-011
- *(workspace)* add fp-frankentui crate scaffold and update workspace dependencies
- complete essence extraction for FP-P2C-006..011 and expand columnar/frame/groupby implementations
- *(types)* add missingness utilities and nanops reductions

### <!-- 1 -->Fixed

- *(fp-types)* semantic_eq bridges all Null kinds (br-frankenpandas-pxv5)
- *(types)* accept legacy "str"/"string" serde aliases for Utf8 DType
- *(hygiene)* apply rustfmt and clippy digit grouping fixes
- *(frame)* preserve object-like constructor scalars (frankenpandas-y2c)
- *(types)* reject string numeric common dtype mixes (frankenpandas-e79)
- *(fp-types)* check identity cast before missing-value branch in cast_scalar_owned
- *(fp-types)* preserve dtype in nanmin/nanmax instead of collapsing to Float64
- *(docs)* fix misaligned right-side borders in all 3 ASCII art diagrams

### <!-- 2 -->Performance

- *(columnar)* eliminate redundant identity casts in scalar coercion hot path

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
- *(fp-types)* add crate-level rustdoc summarizing value-types (br-dagt / fd90.61)
- surface Arrow IPC stream as 8th IO format (br-frankenpandas-zbz5)
- per-crate README.md for each fp-* crate (br-frankenpandas-kw5q)
- close row MultiIndex epic (br-frankenpandas-1zzp)
- align conformance claim to actual 430+ packets / 1249 fixtures (br-frankenpandas-zgqj)
- scope SQL parity claim to SQLite-only (br-frankenpandas-m3e8)
- link row MultiIndex roadmap to epic (br-frankenpandas-0yz7)
- split MultiIndex capability into Row vs Column (br-frankenpandas-0yz7)
- qualify drop-in positioning until PyO3 ships (br-frankenpandas-diic)
- expand README to 1574 lines, de-slopify all AI writing artifacts
- expand README to 1440+ lines with missing data, coercion, element-wise ops, selection, introspection
- expand README to 1200+ lines with optimization catalog, error architecture, constructors, merge options
- expand README to 1000+ lines with recipes, deep dives, roadmap
- expand README with deep technical content (+300 lines)
- complete README.md rewrite reflecting current project state
- update README with project status and architecture overview

### <!-- 5 -->Testing

- *(fuzz)* add semantic_eq invariants target (br-frankenpandas-myra)

### Licensing

- adopt MIT + OpenAI/Anthropic rider across workspace and crates
