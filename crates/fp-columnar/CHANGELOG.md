# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/fp-columnar-v0.1.0) - 2026-04-26

### <!-- 0 -->Added

- *(fp-frame)* add where_mask_df_other to mirror mask_df_other (br-df9v7 / fd90.138)
- *(frankenpandas)* re-export rusqlite under sql-sqlite feature (br-r34z9 / fd90.130)
- *(frankenpandas)* expand prelude with path-based IO + add Path import to Recipes (br-m3zib / fd90.125)
- *(fp-columnar)* add sparse column storage (frankenpandas-0xcm.1)
- *(io)* gate sqlite sql backend (br-frankenpandas-fd90)
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

- *(dtype)* wire sparse marker through crate policies (frankenpandas-0xcm)
- *(fp-columnar)* Column.combine honors fill_value=None null propagation (frankenpandas-x0b5)
- *(frame)* preserve object-like constructor scalars (frankenpandas-y2c)
- *(fp-columnar)* preserve int64 for mod/floordiv when no zero divisors
- integer precision in columnar ops, N-way concat alignment, and merge_asof direction
- *(docs)* fix misaligned right-side borders in all 3 ASCII art diagrams

### <!-- 2 -->Performance

- *(columnar)* eliminate redundant identity casts in scalar coercion hot path

### <!-- 3 -->Changed

- *(fp-frame)* convert where_mask_df_other to delegator over where_cond_df (br-a2sck / fd90.139)

### <!-- 4 -->Documentation

- *(readme)* fix grossly stale FP-P2D packet range/count (br-78ttq / fd90.149)
- *(readme)* correct FAQ Column memory claim — Vec<Scalar>, not typed arrays (br-z2o49 / fd90.148)
- *(readme)* replace dead DuplicateIndexUnsupported with real FrameError wrappers (br-hb9xg / fd90.140)
- *(readme)* note merge_asof returns MergedDataFrame in recipe (br-x8r65 / fd90.137)
- *(readme)* fix where_mask_df / mask_df arg types (br-fjiep / fd90.136)
- *(readme)* correct SeriesGroupBy method list (br-l51hq / fd90.135)
- *(readme)* fix pipe chain — query returns ExprError, not FrameError (br-uukfh / fd90.134)
- *(readme)* replace phantom join/aggregation-specific variant claims (br-unvl4 / fd90.133)
- *(readme)* fix phantom ExprError::UnknownColumn variant (br-ouqtz / fd90.132)
- *(readme)* fix ColumnError row — wrong casing + phantom variant (br-r4ar5 / fd90.131)
- *(readme)* show LengthMismatch as struct variant in error table (br-4r5t5 / fd90.124)
- *(readme)* supply periods arg to df.pct_change() (br-derlf / fd90.123)
- *(readme)* supply args to df.duplicated()/drop_duplicates() (br-kernh / fd90.122)
- *(readme)* wrap melt var_name/value_name in Some() (br-k1b4n / fd90.120)
- *(readme)* fix df.xs("string") — needs &IndexLabel (br-pizo3 / fd90.119)
- *(readme)* fix df.crosstab(str, str) — associated fn on &Series (br-xdolc / fd90.118)
- *(readme)* fix iloc(i64) → iloc(&[i64]) + add ? to head/tail (br-qhlg6 / fd90.117)
- *(readme)* fix index.drop_duplicates(keep) wrong fn + bogus ? (br-uj2np / fd90.116)
- *(readme)* fix sample/sample_weights signatures (br-gdrz5 / fd90.115)
- *(readme)* add missing margins:bool to pivot_table_with_margins[_name] (br-rzdtb / fd90.114)
- *(readme)* fix merge_with_options signature/struct/return type (br-nnjnl / fd90.113)
- *(readme)* fix df.clip + df.replace signatures (br-b7yru / fd90.112)
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
- *(fp-columnar)* add crate-level rustdoc (br-v7oq / fd90.62)
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
