# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/fp-index-v0.1.0) - 2026-04-24

### <!-- 0 -->Added

- *(io)* gate sqlite sql backend (br-frankenpandas-fd90)
- *(row-multiindex)* add tuple lookup and xs APIs (br-frankenpandas-1zzp.3)
- *(fp-frame)* land row_multiindex DataFrame field (br-frankenpandas-1zzp.1)
- *(fp-index)* add Index.asof/searchsorted/memory_usage/nlevels parity (frankenpandas-gsq6)
- *(fp-index)* add Index.to_list/format/putmask parity (frankenpandas-h4xc)
- *(fp-index)* add value_counts parity (frankenpandas-k659)
- *(fp-index)* add Index equals/identical/value_counts/shift/any/all parity (frankenpandas-3arq)
- *(fp-index)* add Index insert/delete/append/repeat/dropna parity (frankenpandas-wss2)
- *(fp-index)* add MultiIndex.duplicated/is_unique parity (frankenpandas-7zzl)
- *(fp-index)* add MultiIndex.isin tuple/level parity (frankenpandas-ajwy)
- *(fp-index)* add multiindex get_indexer_non_unique (frankenpandas-itdh)
- *(fp-frame)* add to_markdown tablefmt options (frankenpandas-ot0k)
- *(frame/groupby/index)* nunique/mode dropna, sort=False, drop_duplicates, NaT
- *(conformance)* IO round-trip fixture ops + reshape/pivot oracle coverage
- *(frame)* implement DataFrame.melt with mixed-type numeric promotion
- add mod/pow/floordiv operators, row-wise apply, and extensive conformance fixes
- *(fp-index)* add MultiIndex.reorder_levels() (frankenpandas-sa6)
- *(fp-index)* add MultiIndex foundation for hierarchical indexing (frankenpandas-uoq)
- *(expr,frame)* add @local variable bindings to expression engine and expand DataFrame operations
- *(fp-frame,fp-index)* add DataFrame.astype, astype_safe, rolling_with_center, Index.names/to_flat_index
- *(fp-frame,fp-index)* add Index.name, map_series, resample extras, apply result_type, groupby rolling/resample
- *(conformance)* add --python-bin CLI flag and join benchmark binary
- add Series convert_dtypes/map_with_na_action/case_when, Index utility methods, and DataFrame extension traits
- massive Series/DataFrame/GroupBy API expansion, merge_asof, and EWM windows
- *(phase2c)* expand compat-closure evidence packs FP-P2C-006 through FP-P2C-011
- *(workspace)* add fp-frankentui crate scaffold and update workspace dependencies
- complete essence extraction for FP-P2C-006..011 and expand columnar/frame/groupby implementations
- *(index)* implement leapfrog triejoin for multi-way index alignment
- *(index)* complete pandas Index model with set ops, dedup, and slicing
- *(frame)* add Series.align(), combine_first(), reindex() with join mode support
- series arithmetic, constructors, join/concat, and alien artifact enhancements

### <!-- 1 -->Fixed

- *(fp-index)* OnceCell → OnceLock for Send+Sync contract (br-frankenpandas-i3t8)
- *(api)* gate all pub error enums with #[non_exhaustive] (br-frankenpandas-tne4)
- *(fp-index)* route align_union through align_non_unique when either side has duplicates
- *(docs)* fix misaligned right-side borders in all 3 ASCII art diagrams

### <!-- 4 -->Documentation

- surface Arrow IPC stream as 8th IO format (br-frankenpandas-zbz5)
- per-crate README.md for each fp-* crate (br-frankenpandas-kw5q)
- *(api)* gate rustdoc and panic contracts (br-frankenpandas-7cfm)
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

- *(index)* complete AG-11-T leapfrog triejoin test plan

### Licensing

- adopt MIT + OpenAI/Anthropic rider across workspace and crates

### Style

- *(fp-index)* cargo fmt collapse of single-assert_eq calls in index test module
