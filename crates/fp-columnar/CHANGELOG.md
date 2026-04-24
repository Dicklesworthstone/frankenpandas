# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/fp-columnar-v0.1.0) - 2026-04-24

### <!-- 0 -->Added

- *(fp-frame)* land row_multiindex DataFrame field (br-frankenpandas-1zzp.1)
- *(fp-columnar)* add searchsorted sorter parity (br-frankenpandas-mr3a)
- *(fp-columnar)* add first/last/count_matching/zip_with/iter_enumerate (frankenpandas-k9g9)
- *(fp-columnar)* add Column iter/to_vec/missing-predicates/apply_bool (frankenpandas-2nvj)
- *(fp-columnar)* add arraylike searchsorted parity (frankenpandas-21gk)
- *(fp-columnar)* add factorize option parity (frankenpandas-if8h)
- *(fp-columnar)* add pct_change fill parity (frankenpandas-s197)
- *(fp-columnar)* add nunique dropna parity (frankenpandas-iwpw)
- *(fp-columnar)* add equals/dot/fillna_with_column/divmod parity (frankenpandas-6v8a)
- *(fp-columnar)* expand ValidityMask surface (frankenpandas-dklr)
- *(fp-columnar)* add where_cond_series/mask_series/replace_values/nonzero (frankenpandas-s4kf)
- *(fp-columnar)* add nsmallest_keep/nlargest_keep with explicit tie policy (frankenpandas-2j2j)
- *(fp-columnar)* add diff_valid/sample/first_valid/last_valid (frankenpandas-75el)
- *(fp-columnar)* add Column.rolling_window_sum parity (frankenpandas-6919)
- *(fp-columnar)* add isnull/notnull/var/std/sem/skew/kurt/ptp parity (frankenpandas-dzxr)
- *(fp-columnar)* add nunique/any/all/is_unique/has_duplicates/pct_change parity (frankenpandas-tuj4)
- *(fp-columnar)* add describe/combine/apply_float/hist_counts parity (frankenpandas-6goz)
- *(fp-columnar)* add argmin/argmax/is_monotonic/combine_first/clip_lower/clip_upper (frankenpandas-ojnj)
- *(fp-columnar)* add Column interpolate/drop_duplicates/compare/map parity (frankenpandas-8y97)
- *(fp-columnar)* add Column quantile/mode/memory_usage parity (frankenpandas-4mts)
- *(fp-columnar)* add Column rank + searchsorted parity (frankenpandas-sqgz)
- *(fp-columnar)* add Column.astype parity (frankenpandas-0kb3)
- *(fp-columnar)* add Column nlargest/nsmallest parity (frankenpandas-vkdg)
- *(fp-columnar)* add Column where_cond/mask parity (frankenpandas-osly)
- *(fp-columnar)* add Column aggregation helpers (frankenpandas-r2eu)
- *(fp-columnar)* add Column sort/argsort/diff/duplicated/between parity (frankenpandas-a1d1)
- *(fp-columnar)* add value_counts parity (frankenpandas-7imk)
- *(fp-columnar)* add Column abs/shift/clip/round/isin parity (frankenpandas-4j5a)
- *(fp-columnar)* add reverse/head/tail/cumulative/unique parity (frankenpandas-d0k6)
- *(fp-columnar)* add Column take/slice/concat/repeat parity (frankenpandas-w3ji)
- *(fp-frame)* report categorical series dtype (frankenpandas-fq5u)
- *(fp-frame)* add to_markdown tablefmt options (frankenpandas-ot0k)
- *(frame)* implement DataFrame.melt with mixed-type numeric promotion
- *(fp-frame)* rank method/na_option validation, mode edge cases, merge_asof + rank conformance
- add mod/pow/floordiv operators, row-wise apply, and extensive conformance fixes
- *(conformance)* add --python-bin CLI flag and join benchmark binary
- *(phase2c)* expand compat-closure evidence packs FP-P2C-006 through FP-P2C-011
- *(workspace)* add fp-frankentui crate scaffold and update workspace dependencies
- complete essence extraction for FP-P2C-006..011 and expand columnar/frame/groupby implementations
- series arithmetic, constructors, join/concat, and alien artifact enhancements
- *(columnar)* extend columnar storage engine with new data types
- *(conformance)* expand differential conformance harness with pandas parity testing

### <!-- 1 -->Fixed

- *(fp-columnar)* Column.combine honors fill_value=None null propagation (frankenpandas-x0b5)
- *(frame)* preserve object-like constructor scalars (frankenpandas-y2c)
- *(fp-columnar)* preserve int64 for mod/floordiv when no zero divisors
- integer precision in columnar ops, N-way concat alignment, and merge_asof direction
- *(docs)* fix misaligned right-side borders in all 3 ASCII art diagrams

### <!-- 2 -->Performance

- *(columnar)* eliminate redundant identity casts in scalar coercion hot path

### <!-- 4 -->Documentation

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

- *(fp-columnar)* clear combine UBS blockers (frankenpandas-dsie)

### Licensing

- adopt MIT + OpenAI/Anthropic rider across workspace and crates

### Style

- *(fp-columnar,fp-frame,fp-io)* clippy and rustfmt cleanup
