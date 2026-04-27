# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/fp-runtime-v0.1.0) - 2026-04-27

### <!-- 0 -->Added

- *(fp-frame)* add where_mask_df_other to mirror mask_df_other (br-df9v7 / fd90.138)
- *(frankenpandas)* re-export rusqlite under sql-sqlite feature (br-r34z9 / fd90.130)
- *(frankenpandas)* expand prelude with path-based IO + add Path import to Recipes (br-m3zib / fd90.125)
- *(io)* gate sqlite sql backend (br-frankenpandas-fd90)
- *(fp-frame)* land row_multiindex DataFrame field (br-frankenpandas-1zzp.1)
- *(fp-frame)* add to_markdown tablefmt options (frankenpandas-ot0k)
- *(conformance)* add --python-bin CLI flag and join benchmark binary
- *(phase2c)* expand compat-closure evidence packs FP-P2C-006 through FP-P2C-011
- *(workspace)* add fp-frankentui crate scaffold and update workspace dependencies
- *(conformance,runtime)* implement ASUPERSYNC governance gates, property tests, and runtime skeleton
- *(runtime,conformance)* implement asupersync module skeleton with governance gates and property tests
- expand groupby aggregation, join operations, and conformance testing
- *(conformance)* expand differential conformance harness with pandas parity testing

### <!-- 1 -->Fixed

- *(docs)* fix misaligned right-side borders in all 3 ASCII art diagrams
- *(runtime)* change RaptorQ placeholder scrub status from "ok" to "placeholder"

### <!-- 3 -->Changed

- *(fp-frame)* convert where_mask_df_other to delegator over where_cond_df (br-a2sck / fd90.139)

### <!-- 4 -->Documentation

- *(readme)* use new Scalar From .into() ergonomics in Quick Start + case_when
- *(readme)* replace fictional compute_ratio_column with inline expression (br-cz3gf / fd90.180)
- *(readme)* fix df.dtypes() return type + missing ? (br-jff4r / fd90.175)
- *(readme)* fix astype-to-Utf8 example — to-Utf8 cast unsupported (br-um5fl / fd90.174)
- *(readme)* fix select_dtypes(strs) → select_dtypes_by_name (br-rb1gy / fd90.170)
- *(readme)* fix df.column(name)? — column returns Option (br-sjeh1 / fd90.169)
- *(readme)* replace fictional compute_weight(i) with inline expression (br-vc9cp / fd90.162)
- *(readme)* bump fp-frame test count 1,433 → 1,434 (br-126io / fd90.151)
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
- *(fp-runtime)* add crate-level rustdoc (br-6l9t / fd90.64)
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

- *(frankenpandas)* add Missing Data Handling test + fix README dropna_with_threshold arity (br-jl1go / fd90.172)

### Licensing

- adopt MIT + OpenAI/Anthropic rider across workspace and crates

### Runtime/conformance

- cap decode proofs and enforce sidecar bounds (bd-3sv5)

### Runtime/docs

- harden prior bounds and sync placeholder scrub semantics (bd-234k, bd-2y32)
