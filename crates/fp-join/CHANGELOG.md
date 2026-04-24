# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/fp-join-v0.1.0) - 2026-04-24

### <!-- 0 -->Added

- *(fp-frame)* land row_multiindex DataFrame field (br-frankenpandas-1zzp.1)
- *(fp-frame)* add to_markdown tablefmt options (frankenpandas-ot0k)
- *(frame)* implement DataFrame.melt with mixed-type numeric promotion
- *(fp-join)* merge_asof tolerance/by/allow_exact_matches parameters
- add mod/pow/floordiv operators, row-wise apply, and extensive conformance fixes
- *(conformance)* add --python-bin CLI flag and join benchmark binary
- add Series convert_dtypes/map_with_na_action/case_when, Index utility methods, and DataFrame extension traits
- massive Series/DataFrame/GroupBy API expansion, merge_asof, and EWM windows
- *(fp-frame)* implement 15 new Series methods, 4 new DataFrame methods, and fix pandas conformance in groupby/types
- *(phase2c)* expand compat-closure evidence packs FP-P2C-006 through FP-P2C-011
- *(workspace)* add fp-frankentui crate scaffold and update workspace dependencies
- series arithmetic, constructors, join/concat, and alien artifact enhancements
- expand groupby aggregation, join operations, and conformance testing
- *(join)* add bumpalo arena allocator path for join intermediate vectors

### <!-- 1 -->Fixed

- *(hygiene)* apply rustfmt to fp-frame and fp-join
- integer precision in columnar ops, N-way concat alignment, and merge_asof direction
- *(docs)* fix misaligned right-side borders in all 3 ASCII art diagrams

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

- *(fp-join)* add merge_asof edge-case tests (frankenpandas-7x9)

### Bd-14fa

- add DataFrame merge suffix tuple parity and collision semantics

### Bd-21wg

- add cross join semantics to fp-join

### Bd-lvj5

- add DataFrame merge sort flag parity and FP-P2D-037 coverage

### Conformance

- broaden dataframe merge/constructor parity surface and packet coverage

### Licensing

- adopt MIT + OpenAI/Anthropic rider across workspace and crates

### Style

- normalize rustfmt formatting across fp-expr, fp-frame, fp-groupby, fp-io, fp-join
