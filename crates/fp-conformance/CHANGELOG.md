# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/fp-conformance-v0.1.0) - 2026-04-23

### <!-- 0 -->Added

- *(conformance)* surface live-oracle aggregate in ci gates (br-frankenpandas-qi6y)
- *(io)* round-trip row multiindex across formats (br-frankenpandas-1zzp.4)
- *(fp-frame)* land row_multiindex DataFrame field (br-frankenpandas-1zzp.1)
- *(fp-frame)* add categorical value_counts parity (frankenpandas-9v09)
- *(fp-io)* add combined parse_dates parity (frankenpandas-b8ov)
- *(fp-frame)* report categorical series dtype (frankenpandas-fq5u)
- *(fp-frame)* add split expand n padding parity (frankenpandas-hlbl)
- *(io)* CSV parse_dates support with mixed-timezone strict fixture (FP-P2D-429)
- *(io)* CSV true_values / false_values parity with pandas (FP-P2D-426)
- *(fp-io)* add read_csv decimal separator parity (frankenpandas-b7yx)
- *(fp-io)* add read_csv on_bad_lines parity (frankenpandas-rzbu)
- *(fp-frame)* add observed=true categorical groupby parity (frankenpandas-es0r)
- *(fp-frame)* add dt.to_pydatetime warn false parity (frankenpandas-925a)
- *(fp-frame)* add dataframe memory_usage option parity (frankenpandas-g9aq)
- *(fp-frame)* add normalized series value_counts parity (frankenpandas-zm86)
- *(fp-frame)* add descending groupby ngroup parity (frankenpandas-5g0n)
- *(fp-frame)* add dataframe corr numeric_only parity (frankenpandas-1zox)
- *(fp-frame)* add dt.to_timestamp how end parity (frankenpandas-rvow)
- *(fp-frame)* align Series.clip array bounds by index (frankenpandas-32xe)
- *(fp-frame)* add set_index verify_integrity parity (frankenpandas-ceii)
- *(fp-frame)* add series str.wrap drop_whitespace parity (frankenpandas-a5dg)
- *(fp-frame)* add dataframe explode ignore_index parity (frankenpandas-6z81)
- *(fp-frame)* add dataframe compare result_names parity (frankenpandas-j7hf)
- *(fp-frame)* validate categorical from_codes parity (frankenpandas-4b3s)
- *(fp-frame)* add str.translate deletion parity (frankenpandas-ox28)
- *(conformance)* add Series.map na_action ignore parity (frankenpandas-s73j)
- *(fp-frame)* add to_markdown tablefmt options (frankenpandas-ot0k)
- *(groupby)* add descending cumcount parity (frankenpandas-nmvv)
- *(fp-frame)* add Series.dt.nanosecond parity (frankenpandas-zhea)
- *(fp-frame)* Series.dt.microsecond accessor + FP-P2D-415 parity fixture
- *(conformance)* add FP-P2D-164 DataFrame memory_usage packet coverage
- *(conformance)* add FP-P2D-163 DataFrame value_counts packet coverage
- *(conformance)* add FP-P2D-162 DataFrame quantile packet coverage
- *(conformance)* add FP-P2D-161 DataFrame nunique packet coverage
- *(conformance)* add FP-P2D-159/160 DataFrame any/all packet coverage
- *(conformance)* add FP-P2D-158 DataFrame median packet coverage
- *(conformance)* add FP-P2D-156/157 DataFrame min/max packet coverage
- *(conformance)* add FP-P2D-154/155 DataFrame std/var packet coverage
- *(conformance)* add FP-P2D-153 DataFrame mean packet coverage
- *(conformance)* add FP-P2D-152 DataFrame sum packet coverage
- *(conformance)* add FP-P2D-151 DataFrame prod packet coverage
- *(conformance)* add FP-P2D-150 DataFrame kurtosis packet coverage
- *(conformance)* add FP-P2D-149 DataFrame sem/skew packet coverage
- *(conformance)* add FP-P2D-148 DataFrame idxmin/idxmax packet coverage
- *(conformance)* add FP-P2D-147 DataFrame cov packet coverage
- *(conformance)* add FP-P2D-146 DataFrame corr packet coverage
- *(conformance)* add FP-P2D-056 pct_change periods=2 packet coverage
- *(conformance)* add FP-P2D-145 DataFrame describe packet coverage
- *(conformance)* add FP-P2D-144 DataFrame shift axis=1 packet coverage
- *(conformance)* add FP-P2D-143 DataFrame mask DataFrame-other packet coverage
- *(conformance)* add FP-P2D-142 DataFrame where DataFrame-other packet coverage
- *(conformance)* add FP-P2D-141 DataFrame mask packet coverage
- *(conformance)* add FP-P2D-140 DataFrame where packet coverage
- *(conformance)* add FP-P2D-139 DataFrame replace packet coverage
- *(conformance)* add FP-P2D-138 DataFrame drop_columns packet coverage
- *(conformance)* add FP-P2D-137 DataFrame reindex_columns packet coverage
- *(conformance)* add FP-P2D-136 DataFrame reindex packet coverage
- *(conformance)* add FP-P2D-135 DataFrame rename_columns packet coverage
- *(conformance)* add FP-P2D-134 DataFrame assign packet coverage
- *(conformance)* add DataFrame insert packet coverage
- *(conformance)* add DataFrame top-N packet coverage
- *(conformance)* add FP-P2D-131 DataFrame transpose packet coverage
- *(conformance)* astype/clip/abs/round DataFrame ops
- *(conformance)* IO round-trip fixture ops + reshape/pivot oracle coverage
- *(fuzz)* add fp-io CSV parser fuzz target with round-trip oracle
- *(groupby)* add any() and all() aggregation with conformance coverage
- *(conformance)* add mixed string/numeric constructor divergence tracking
- *(frame)* implement DataFrame.melt with mixed-type numeric promotion
- *(conformance)* add Series diff, shift, and pct_change conformance packets
- *(fp-conformance)* add series diff/shift/pct_change operations
- *(fp-join)* merge_asof tolerance/by/allow_exact_matches parameters
- *(fp-frame)* rank method/na_option validation, mode edge cases, merge_asof + rank conformance
- add mod/pow/floordiv operators, row-wise apply, and extensive conformance fixes
- *(fp-conformance)* add join/filter performance baselines (frankenpandas-n3t)
- *(fp-conformance)* add DataFrame arithmetic/comparison property tests (frankenpandas-s6d)
- *(fp-io,fp-conformance)* add Excel I/O and property-based fuzz tests (frankenpandas-6s1, frankenpandas-x2n)
- *(fp-frame)* add row-record (matrix_rows) DataFrame.from_records constructor
- *(conformance)* add --python-bin CLI flag and join benchmark binary
- *(conformance)* add FP-P2D-054 duplicate detection and FP-P2D-055 series arithmetic packets
- *(conformance)* add FP-P2D-053 set/reset_index conformance packet
- *(conformance)* add pandas oracle fixture generator and new test cases
- add Series any/all aggregation and DataFrame sort_index/sort_values parity
- *(parity)* add FP-P2D-028 dataframe concat axis=1 coverage (bd-229z)
- *(parity)* add FP-P2D-027 negative-n head/tail coverage (bd-38pw)
- add dataframe selector and head-tail parity packets (bd-2omm bd-32z8)
- *(conformance)* add FP-P2D-022 DataFrame constructor list-like shape taxonomy packet
- *(conformance)* fall back to fixture expectations when live oracle is unavailable
- *(oracle)* harden pandas import fallback and add series_filter/series_head ops
- *(conformance)* expand DataFrame loc/iloc with error-path fixture packets and harness logic
- *(frame,conformance)* expand DataFrame implementation and add loc/iloc fixture packets
- *(conformance)* add DataFrame loc/iloc label-based and positional row selection tests
- implement Series loc/iloc label-based and position-based selection
- *(phase2c)* expand compat-closure evidence packs FP-P2C-006 through FP-P2C-011
- *(workspace)* add fp-frankentui crate scaffold and update workspace dependencies
- *(conformance,runtime)* implement ASUPERSYNC governance gates, property tests, and runtime skeleton
- *(runtime,conformance)* implement asupersync module skeleton with governance gates and property tests
- *(conformance)* add CI gate forensics infrastructure with machine-readable reports
- *(phase2c)* deepen rule/error ledgers for 001-005, add conformance fixtures for 006-011
- *(frame,conformance)* expand frame operations and conformance harness
- complete essence extraction for FP-P2C-006..011 and expand columnar/frame/groupby implementations
- expand groupby aggregation, join operations, and conformance testing
- *(conformance)* expand differential conformance harness with pandas parity testing

### <!-- 1 -->Fixed

- *(api)* gate all pub error enums with #[non_exhaustive] (br-frankenpandas-tne4)
- *(conformance)* require live oracle in CI (br-frankenpandas-d6xa)
- *(fp-io)* restore excel default round-trip (br-frankenpandas-ho6t)
- *(fp-frame)* restore grouped window fallback schema (br-frankenpandas-ch4o)
- *(fp-conformance)* median multikey fixture + drop unused write_excel_bytes import (br-frankenpandas-ebjw)
- *(fp-conformance)* assert_excel_roundtrip writes with index=false (br-frankenpandas-mxho)
- *(fp-conformance)* excel round-trip writes with index=false to match default reader (br-frankenpandas-989b)
- *(fp-conformance)* align FP-P2C-005 groupby_sum_order with pandas default sort (br-frankenpandas-g90b)
- *(fp-conformance)* handle 4 new DataFrameApply* variants in match arms (br-frankenpandas-bmz6)
- *(fp-conformance)* bump unsupported_agg fixture off median (br-frankenpandas-u7n1)
- *(fp-conformance)* make phase2c test lock poison-tolerant (br-frankenpandas-ijpu)
- *(fp-frame)* rolling min min_periods=0 parity (frankenpandas-brti)
- *(str)* column-aware expandtabs + cased-run based title() semantics
- *(conformance)* preserve value_counts tie order
- *(conformance)* correct FP-P2D-150 kurtosis fixture precision
- *(conformance)* use utf8 scalar kind in FP-P2D-144 fixture
- *(oracle)* resolve UBS undefined-name findings in pandas oracle
- *(hygiene)* apply rustfmt and clippy digit grouping fixes
- *(conformance)* align FP-P2D-130 fixture expectations with dtype behavior
- *(conformance)* fix clip null kind expectation and DataFrame op error types
- *(conformance)* correct timedelta fixture expectations to use timedelta64
- *(fp-conformance)* add missing DataFrameAtTime and DataFrameBetweenTime variants to execution matching
- *(frame)* preserve object-like constructor scalars (frankenpandas-y2c)
- *(types)* reject string numeric common dtype mixes (frankenpandas-e79)
- *(conformance)* align series gap packets with pandas NaN semantics (frankenpandas-lo6)
- *(conformance)* correct melt executor import and dispatch wiring
- *(docs)* fix misaligned right-side borders in all 3 ASCII art diagrams
- *(fp-io,fp-conformance)* fix 3 bugs found in code review
- *(conformance)* apply rustfmt brace formatting to csv_round_trip match arm
- *(conformance)* reject sidecars with envelope/packet count mismatches
- *(conformance)* update conformance harness wiring for pandas parity tests

### <!-- 3 -->Changed

- *(conformance)* capture oracle errors as report mismatches instead of propagating

### <!-- 4 -->Documentation

- per-crate README.md for each fp-* crate (br-frankenpandas-kw5q)
- *(api)* gate rustdoc and panic contracts (br-frankenpandas-7cfm)
- close row MultiIndex epic (br-frankenpandas-1zzp)
- align conformance claim to actual 430+ packets / 1249 fixtures (br-frankenpandas-zgqj)
- scope SQL parity claim to SQLite-only (br-frankenpandas-m3e8)
- link row MultiIndex roadmap to epic (br-frankenpandas-0yz7)
- split MultiIndex capability into Row vs Column (br-frankenpandas-0yz7)
- qualify drop-in positioning until PyO3 ships (br-frankenpandas-diic)
- *(conformance)* refresh packet coverage ledger
- *(fp-conformance)* add mandatory conformance harness documentation
- rustfmt pass and new API surface across fp-index, fp-io, fp-join, fp-frame, fp-expr, and frankenpandas
- expand README to 1574 lines, de-slopify all AI writing artifacts
- expand README to 1440+ lines with missing data, coercion, element-wise ops, selection, introspection
- expand README to 1200+ lines with optimization catalog, error architecture, constructors, merge options
- expand README to 1000+ lines with recipes, deep dives, roadmap
- expand README with deep technical content (+300 lines)
- complete README.md rewrite reflecting current project state
- update README with project status and architecture overview

### <!-- 5 -->Testing

- *(fuzz)* parallel DataFrame fuzz target + TSan workflow (br-frankenpandas-bahw)
- *(fuzz)* stateful DataFrame op-chain target (br-frankenpandas-wr9n)
- *(fuzz)* fuzz_sql_read target for fp-io SQL surface (br-frankenpandas-gpxk)
- *(conformance)* reverse-oracle scaffolding (br-frankenpandas-kdwn)
- *(oracle)* add pytest suite for pandas_oracle.py (br-frankenpandas-urhy)
- *(fp-conformance)* extract fuzz seed-fixture tests (br-frankenpandas-lxhr)
- *(fuzz)* parse_expr + query_str + query_str_with_locals targets (br-frankenpandas-jkhg + br-frankenpandas-0iyb)
- *(fuzz)* add regression corpus scaffolding (br-frankenpandas-lvl6)
- *(fp-conformance)* split CI supply-chain policy tests (br-frankenpandas-lxhr)
- *(fp-conformance)* split live oracle report tests (br-frankenpandas-lxhr)
- *(fp-conformance)* split live oracle availability tests (br-frankenpandas-lxhr)
- *(fp-conformance)* split series misc oracle tests (br-frankenpandas-lxhr)
- *(fp-conformance)* split to_datetime variant oracle tests (br-frankenpandas-lxhr)
- *(fp-conformance)* split dataframe series core oracle tests (br-frankenpandas-lxhr)
- *(fp-conformance)* split groupby misc oracle tests (br-frankenpandas-lxhr)
- *(fp-conformance)* split dataframe series selection oracle tests (br-frankenpandas-lxhr)
- *(fp-conformance)* split dataframe merge oracle tests (br-frankenpandas-lxhr)
- *(fp-conformance)* split row multiindex oracle tests (br-frankenpandas-lxhr)
- *(fp-conformance)* split dataframe rolling oracle tests (br-frankenpandas-lxhr)
- *(fp-conformance)* split dataframe resample oracle tests (br-frankenpandas-lxhr)
- *(fp-conformance)* split dataframe apply oracle tests (br-frankenpandas-lxhr)
- *(perf)* gate perf baselines in CI (br-frankenpandas-ing6)
- *(conformance)* pin fixture provenance and stale-fixture gate (br-frankenpandas-boyr)
- *(fuzz)* lock in regression corpus CI (br-frankenpandas-zjme)
- *(conformance)* add row multiindex live oracle coverage (br-frankenpandas-1zzp.5)
- *(conformance)* remove excel index=false escape hatch (br-frankenpandas-arnn)
- *(conformance)* surface case evidence artifacts in G6 output (br-frankenpandas-x4ud)
- *(fuzz)* add rolling window min_periods target (br-frankenpandas-6xub)
- *(fuzz)* add groupby agg dispatch target (br-frankenpandas-aj8a)
- *(fp-conformance)* add grouped resample first/last oracle (br-frankenpandas-icm3)
- *(fuzz)* add pivot_table dispatch target (br-frankenpandas-p762)
- *(fp-conformance)* live oracle for DataFrameGroupBy.rolling std|var (br-frankenpandas-fmmq)
- *(fp-conformance)* wire DataFrameGroupByRolling* into run_fixture_operation
- *(conformance)* extend groupby.rolling live-oracle to min/max/count
- *(conformance)* add DataFrameGroupBy.rolling sum/mean live-oracle coverage
- *(fuzz)* add semantic_eq invariants target (br-frankenpandas-myra)
- *(conformance)* align excel round-trip props (br-frankenpandas-ho6t)
- *(fuzz)* add dataframe eval target (br-frankenpandas-s5cj)
- *(conformance)* emit per-case failure evidence ledgers (br-frankenpandas-x4ud)
- *(fuzz)* add cross-format round-trip parity target (br-frankenpandas-pavu)
- *(fp-conformance)* surface actionable aggregate failures (br-frankenpandas-44vd)
- *(fp-conformance)* add groupby resample live oracle coverage (br-frankenpandas-8slc)
- *(fp-conformance)* add records json parity packet (frankenpandas-vkq4)
- *(to_datetime)* add timestamp-like origin parity (frankenpandas-oxh1)
- *(fp-conformance)* add groupby agg multi parity packet (frankenpandas-od8v)
- *(fp-conformance)* add Series.to_arrow round-trip packet (frankenpandas-02bc)
- *(conformance)* register series_split_df fixture op for str.split(expand=True) parity
- *(fp-conformance)* add arithmetic metamorphic invariants (frankenpandas-yynk)
- *(fp-frame)* add drop_duplicates subset order parity (frankenpandas-r0c1)
- *(fp-conformance)* add dtype-aware series asof parity (frankenpandas-3hz1)
- *(fp-frame)* add unicode removeprefix exactness parity (frankenpandas-88mp)
- *(fp-conformance)* add named-group extract parity packet (frankenpandas-olyb)
- add Series.between left/right parity packets (frankenpandas-81cv)
- add dataframe isna/notna NaN empty-string packets (frankenpandas-87hj)
- *(conformance)* parity packet fixtures for title/regex/split-get/loc/iloc/head/isna edge cases
- *(conformance)* parity packet fixtures for string-method + head/tail edge cases
- *(conformance)* FP-P2D-165/166 head/tail zero-n strict fixtures
- *(conformance)* parity packet fixtures for FP-P2D-051/130/148-168 edge cases
- *(conformance)* oracle + proptest + fixture coverage for new parity work
- *(conformance)* add series rolling window fixtures
- *(conformance)* add dataframe_mode edge fixtures
- *(conformance)* add dataframe_diff periods=2 fixtures
- *(conformance)* add series_sub and series_mul edge fixtures
- *(conformance)* add series_concat edge fixtures
- *(conformance)* add series_sort_index edge fixtures
- *(conformance)* add series_head edge fixtures
- *(conformance)* add groupby multikey aggregate fixtures
- *(conformance)* expand series diff pct change fixtures
- *(conformance)* cover dataframe pivot parity
- *(conformance)* cover dataframe covariance packet gap
- *(conformance)* cover series add packet gap
- *(conformance)* cover rank and mode packet gap
- *(conformance)* cover kurtosis packet gap
- *(conformance)* cover aggregate packets
- *(conformance)* gate FP-P2D-137 packet fixtures
- *(conformance)* pin series_to_timedelta_strings on Timedelta64 output
- *(conformance)* more FP-P2D-130 fixtures + pin series to_timedelta output dtype
- *(conformance)* FP-P2D-130 numeric-transform packet + oracle dtype helpers
- *(conformance)* bump FP-P2D-129 fixture floor from 5 to 9
- *(conformance)* add FP-P2D-129 IO round-trip fixtures + packet filter test
- *(conformance)* add FP-P2D-127 fixture packets for reshape/pivot/rolling/expanding/ewm
- *(proptest)* reconstruct_pct_change_axis1_dataframe helper
- *(conformance)* dataframe groupby ngroup packet (FP-P2D-112)
- *(proptest)* pct_change reconstruction helpers for DataFrame parity
- *(conformance)* dataframe groupby OHLC packet (FP-P2D-110)
- *(fp-conformance)* add end-to-end pandas workflow integration tests (frankenpandas-enl)
- *(fp-conformance)* add property-based IO round-trip tests for SQL, Excel, Feather (frankenpandas-44y)
- *(conformance)* add proptest round-trip properties for Feather, IPC, SQL, and Excel
- *(conformance)* add FP-P2D-040 sort and FP-P2D-041 any/all oracle-backed fixture packets

### <!-- 6 -->CI

- *(security)* add cargo-audit and cargo-deny gates (br-frankenpandas-36qc)

### Bd-14fa

- add DataFrame merge suffix tuple parity and collision semantics

### Bd-1q3z

- add FP-P2D-030 axis0 inner concat parity

### Bd-1qqi

- reject series_join cross and add negative packet coverage

### Bd-5dv8

- add DataFrame merge cross-join parity in conformance + FP-P2D-039 packet

### Bd-lvj5

- add DataFrame merge sort flag parity and FP-P2D-037 coverage

### Bd-s7aq

- add FP-P2D-038 merge_sort parity coverage for index/alias key paths

### Bd-z45l

- add FP-P2D-029 dataframe concat axis=1 inner parity

### Build

- *(ci)* pin dated nightly toolchain (br-frankenpandas-1d9y)

### Conformance

- broaden dataframe merge/constructor parity surface and packet coverage
- enforce sidecar artifact_id packet coherence (bd-3b50)

### Fp-conformance

- add dataframe apply oracle coverage (br-frankenpandas-ikkn)
- add FP-P2D-024 dtype-spec normalization packet (bd-29xq)
- add FP-P2D-023 constructor dtype/copy parity packet (bd-ddib)

### Licensing

- adopt MIT + OpenAI/Anthropic rider across workspace and crates

### Port

- validate malformed Series.dt.to_timestamp periods (frankenpandas-16hq)

### Runtime/conformance

- cap decode proofs and enforce sidecar bounds (bd-3sv5)

### Style

- apply rustfmt cleanup after frankenpandas-16hq
- *(fp-conformance)* rustfmt SeriesToFrame match arm
- *(conformance)* collapse single-expression format! calls onto one line
