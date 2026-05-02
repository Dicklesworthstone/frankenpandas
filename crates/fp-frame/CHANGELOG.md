# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/fp-frame-v0.1.0) - 2026-05-02

### <!-- 0 -->Added

- *(fp-frame)* add pandas-parity df.min/max/std/var/etc. methods (fd90.233 / br-he7yj)
- *(fp-frame)* add DataFrameResample::first/last (fd90.200 / br-kzmxd)
- *(fp-frame)* add where_mask_df_other to mirror mask_df_other (br-df9v7 / fd90.138)
- *(frankenpandas)* re-export rusqlite under sql-sqlite feature (br-r34z9 / fd90.130)
- *(frankenpandas)* expand prelude with path-based IO + add Path import to Recipes (br-m3zib / fd90.125)
- *(fp-frame)* expand groupby apply result shapes (frankenpandas-0kx7)
- *(fp-frame)* add Series sparse accessor (frankenpandas-0xcm.2)
- *(io)* gate sqlite sql backend (br-frankenpandas-fd90)
- *(row-multiindex)* add reshape round-trips (br-frankenpandas-1zzp.6)
- *(row-multiindex)* add tuple lookup and xs APIs (br-frankenpandas-1zzp.3)
- *(fp-frame)* groupby emits real row MultiIndex on multi-key (br-frankenpandas-1zzp.2)
- *(fp-frame)* land row_multiindex DataFrame field (br-frankenpandas-1zzp.1)
- add DataFrame apply prod axis=1 parity (br-frankenpandas-nxm3)
- add DataFrame resample agg prod parity (br-frankenpandas-0u7s)
- add DataFrame resample prod parity (br-frankenpandas-j3h9)
- add DataFrame.apply product alias parity (br-frankenpandas-dgfr)
- add DataFrame.apply nunique parity (br-frankenpandas-bwsx)
- add DataFrame.apply sem axis0 parity (br-frankenpandas-uprj)
- *(fp-frame)* add DataFrame.apply('prod', axis=0) parity (br-frankenpandas-lwu8)
- add pivot_table prod parity (br-frankenpandas-5lro)
- *(fp-frame)* extend DataFrame.resample(freq).agg to std/var/median (frankenpandas-x10m)
- add rolling agg first/last/prod (br-frankenpandas-4ean)
- add simple_grid markdown parity (br-nat2)
- add heavy_grid markdown parity (br-f5kz)
- add outline markdown parity (br-aay2)
- add double_grid markdown parity (br-ic8q)
- add mixed_grid markdown parity (br-ht7i)
- add series reset_index name parity (br-0jmn)
- add fancy_grid to_markdown parity (br-avmx)
- add rounded_grid to_markdown parity (br-143b)
- add pivot_table std-var margins parity (br-kusq)
- add Series.reset_index DataFrame parity (br-52lj)
- *(fp-frame)* add to_markdown tablefmt support (br-frankenpandas-qfah)
- *(fp-frame)* add pivot_table median std var aggfuncs (br-frankenpandas-jj8w)
- *(fp-frame)* support first margins in pivot_table (br-frankenpandas-rnzg)
- *(fp-frame)* add minute/hour to_datetime units (br-frankenpandas-et93)
- *(fp-frame)* add groupby transform list parity (br-frankenpandas-88cw)
- *(fp-frame)* add pivot_table dropna parity (frankenpandas-o8a6)
- *(fp-frame)* add groupby transform closure parity (frankenpandas-no1t)
- *(fp-frame)* add groupby agg dict-of-list parity (frankenpandas-715r)
- *(fp-frame)* add pivot_table aggfunc-dict parity (frankenpandas-7n5h)
- *(fp-frame)* add DataFrame.iat + DataFrame.set_axis parity (frankenpandas-fga7)
- *(fp-frame)* add user-function DataFrame.apply parity (frankenpandas-z7k1)
- *(fp-frame)* add EWM corr parity (frankenpandas-5agu)
- *(fp-frame)* add DataFrame.at + Series.map_callable parity (frankenpandas-xi36)
- *(fp-frame)* add DataFrame.value_counts_map per-column series (frankenpandas-1qof)
- *(fp-frame)* add DataFrame.quantile_axis1/argmin_axis1/argmax_axis1 (frankenpandas-0gc3)
- *(fp-frame)* add DataFrame.expanding().skew()/kurt() parity (frankenpandas-1c8t)
- *(fp-frame)* add DataFrame.ewm().sum() parity (frankenpandas-fcck)
- *(fp-frame)* add Series.ewm().sum()/cov() parity (frankenpandas-8dru)
- *(fp-frame)* add non-numeric searchsorted parity (frankenpandas-yrhm)
- *(fp-frame)* add Series.reset_index parity (frankenpandas-zuvm)
- *(fp-frame)* add DataFrame.size property parity (frankenpandas-wvbi)
- *(fp-frame)* add Series.ndim/size/axes/empty properties (frankenpandas-n0sh)
- *(fp-frame)* to_dict dict dup-index rejection + DataFrame.empty property (frankenpandas-rs33)
- *(fp-frame)* fix split to_dict payload parity (frankenpandas-ytn5)
- *(fp-frame)* add to_dict list parity (frankenpandas-0zpj)
- *(fp-frame)* add categorical value_counts parity (frankenpandas-9v09)
- *(fp-frame)* integrate ohlc column MultiIndex metadata (frankenpandas-kqjk)
- *(fp-frame)* report categorical series dtype (frankenpandas-fq5u)
- *(fp-frame)* expose groupby ohlc column multiindex helper (frankenpandas-kr5d)
- *(fp-frame)* add split expand n padding parity (frankenpandas-hlbl)
- *(fp-frame)* str.match/fullmatch case/na option parity (frankenpandas-13ph)
- *(fp-frame)* add Series.expanding().corr/cov(other) parity (frankenpandas-xarz)
- *(fp-frame)* add DataFrame.rolling().cov_with/corr_with (frankenpandas-tkrv)
- *(fp-frame)* add str.replace n/case/regex option parity (frankenpandas-ghqp)
- *(fp-frame)* add str.split expand n parameter parity (frankenpandas-2gyf)
- *(fp-frame)* str.extract named-group series name parity (frankenpandas-1r4h)
- *(fp-frame)* str.extractall named-group column parity (frankenpandas-6f3f)
- *(fp-frame)* add str.contains case/na/regex option parity (frankenpandas-5xq7)
- *(fp-frame)* add str.startswith/endswith na parameter parity (frankenpandas-dvzt)
- *(fp-frame)* add get_dummies drop_first/dummy_na parity (frankenpandas-1w18)
- *(fp-frame)* add Series.pct_change fill_method/limit parity (frankenpandas-gwga)
- *(fp-frame)* add observed=true categorical groupby parity (frankenpandas-es0r)
- *(fp-frame)* add Series.divmod/rdivmod parity (frankenpandas-nk83)
- *(fp-frame)* add Series.has_duplicates parity (frankenpandas-ijl4)
- *(fp-frame)* add select_dtypes string-alias parity (frankenpandas-sa33)
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
- *(fp-frame)* add to_markdown tablefmt options (frankenpandas-ot0k)
- *(groupby)* add descending cumcount parity (frankenpandas-nmvv)
- *(fp-frame)* add Series.dt.nanosecond parity (frankenpandas-zhea)
- *(fp-frame)* Series.dt.microsecond accessor + FP-P2D-415 parity fixture
- *(str)* proper Unicode casefold + run-based swapcase for StringAccessor
- *(frame/groupby/index)* nunique/mode dropna, sort=False, drop_duplicates, NaT
- *(conformance)* IO round-trip fixture ops + reshape/pivot oracle coverage
- add Series.is_monotonic() alias for pandas parity
- implement missing parity features
- *(groupby)* add any() and all() aggregation with conformance coverage
- add any and all for DataFrameGroupBy
- *(fp-frame)* add quantile for DataFrameGroupBy
- *(fp-frame)* add take_rows and take_columns to DataFrame
- *(fp-frame)* add periods and limits args to GroupBy ops & rank/shift_axis1 for DataFrame
- *(frame)* implement DataFrame.melt with mixed-type numeric promotion
- *(fp-frame)* rank method/na_option validation, mode edge cases, merge_asof + rank conformance
- add mod/pow/floordiv operators, row-wise apply, and extensive conformance fixes
- *(fp-frame)* add DataFrameGroupBy.agg_named() for named aggregation (frankenpandas-ka5)
- *(fp-frame)* add pd.to_timedelta() and timedelta_total_seconds() (frankenpandas-907)
- *(fp-frame)* add pd.to_datetime() equivalent for datetime parsing (frankenpandas-rkr)
- *(fp-frame)* add to_dict tight orient, to_series_dict, fix label display (frankenpandas-q4f)
- *(fp-frame)* integrate MultiIndex with DataFrame set_index/reset_index (frankenpandas-ujl)
- *(fp-frame)* add Categorical dtype support as Series metadata (frankenpandas-qej)
- *(fp-frame)* add DataFrame to_json orient parity (bd-cvoa)
- *(fp-frame)* expand timezone localization with series-level ambiguous/nonexistent policies
- *(fp-frame)* add chrono-tz support and rewrite timezone handling
- *(fp-frame)* add row-record (matrix_rows) DataFrame.from_records constructor
- *(expr,frame)* add @local variable bindings to expression engine and expand DataFrame operations
- *(fp-frame)* add timezone operations and DataFrame.from_tuples_with_index
- *(fp-frame)* add skipna aggregates, first/last offset, date arithmetic helpers
- *(fp-frame,fp-index)* add DataFrame.astype, astype_safe, rolling_with_center, Index.names/to_flat_index
- *(fp-frame,fp-index)* add Index.name, map_series, resample extras, apply result_type, groupby rolling/resample
- *(fp-frame)* add rolling.agg, to_numpy_2d, dt.to_timestamp
- *(fp-frame)* add drop_columns, clip_with_series, nlargest_multi
- *(fp-frame)* add SeriesGroupBy.agg, reindex_columns, from_tuples
- *(fp-frame)* add dt.round, SeriesGroupBy var/median/prod, Series.replace_regex
- *(fp-frame)* add replace_map, apply_fn_na, resample agg multi
- *(fp-frame)* add pivot_table_fill, count_na, swaplevel
- *(fp-frame)* add dt.ceil/floor, groupby dropna, sample_weights
- *(fp-frame)* add where_cond_series, mask_series, fillna_limit
- *(fp-frame)* add slice_replace, describe_dtypes, and value_counts_subset methods
- *(fp-frame)* add truncate, replace_regex, and to_string_truncated
- *(fp-frame)* add reindex_fill, rolling center, and pivot_table_multi_values
- *(fp-frame)* add pow_df, Series.agg, and Series.groupby
- *(frame)* add Series.agg() multi-function aggregation and Series.groupby()
- *(fp-frame)* add expanding skew/kurt, to_records, and fill arithmetic
- *(fp-frame)* add expanding skewness and kurtosis methods
- *(fp-frame)* add rolling skew/kurt, Series.item, and DataFrame.rename_index
- *(fp-frame)* add DataFrame fill arithmetic, extract_to_frame, and margins_name
- *(fp-frame)* add corrwith_axis and map_with_default
- *(fp-frame)* add DataFrame.std_agg_ddof and var_agg_ddof
- *(fp-frame)* add Series.var_ddof and std_ddof for configurable degrees of freedom
- *(fp-frame)* add DataFrame.combine for element-wise binary operations
- *(fp-frame)* add Series.duplicated_keep with DuplicateKeep parameter
- *(fp-frame)* add value_counts_bins and pivot_table_multi_agg
- *(fp-frame)* add Series.value_counts_bins for binned frequency counting
- *(fp-frame)* add Series.describe, drop_duplicates_keep, and DataFrame.to_csv_options
- *(fp-frame)* add corr_min_periods and cov_min_periods for DataFrame
- *(fp-frame)* add assign_fn, applymap_na_action, and isin_dict methods
- *(fp-frame)* add pivot_table_with_margins, interpolate_method, and fillna_dict
- *(fp-frame)* add reindex_with_method and groupby sort parameter
- *(frame)* add na_position control, ignore_index concat, fillna_dict, replace_dict, and groupby agg_multi
- *(frame)* add loc_bool_series and iloc_bool_series accessors for Series and DataFrame
- *(frame)* add loc/iloc boolean mask, slice, and row accessors for Series and DataFrame; add groupby as_index=false support
- *(conformance)* add --python-bin CLI flag and join benchmark binary
- expand Series/DataFrame API with comparison ops, get_dummies, rolling corr/cov, string accessor extensions, utility functions, and DatetimeAccessor enhancements
- add Series convert_dtypes/map_with_na_action/case_when, Index utility methods, and DataFrame extension traits
- massive Series/DataFrame/GroupBy API expansion, merge_asof, and EWM windows
- major DataFrame frame implementation rewrite and expression engine
- *(fp-frame)* add stack/unstack, closure-based apply_fn, and map_values
- *(fp-frame)* add DataFrame groupby integration, sample, and info
- *(fp-frame)* add Series str accessor with 15 string operations
- *(fp-frame)* add agg, applymap, transform, corr, cov, nlargest, nsmallest, reindex
- *(fp-frame)* add rank, melt, and pivot_table methods
- *(fp-frame)* add rolling/expanding window operations for Series
- *(fp-frame,fp-groupby)* add DataFrame where/mask/iterrows/itertuples/items/assign/pipe methods
- *(fp-frame)* add Series idxmin/idxmax, nlargest/nsmallest, and pct_change with tests
- *(fp-frame)* add Series conditional/membership methods (where_cond, mask, isin, between)
- *(fp-frame)* add row-wise median to DataFrame.apply and return typed Scalars
- *(fp-frame)* add row-wise var/std to DataFrame.apply, refine clip and test precision
- *(fp-frame)* implement 15 new Series methods, 4 new DataFrame methods, and fix pandas conformance in groupby/types
- *(frame)* support duplicate index labels when all inputs are exactly aligned
- add Series any/all aggregation and DataFrame sort_index/sort_values parity
- *(parity)* add FP-P2D-028 dataframe concat axis=1 coverage (bd-229z)
- *(parity)* add FP-P2D-027 negative-n head/tail coverage (bd-38pw)
- add dataframe selector and head-tail parity packets (bd-2omm bd-32z8)
- *(frame,conformance)* add boolean mask type validation and error-path conformance support
- *(frame,conformance)* expand DataFrame implementation and add loc/iloc fixture packets
- *(frame)* implement DataFrame loc/iloc label-based and position-based row selection
- implement Series loc/iloc label-based and position-based selection
- *(phase2c)* expand compat-closure evidence packs FP-P2C-006 through FP-P2C-011
- *(workspace)* add fp-frankentui crate scaffold and update workspace dependencies
- *(frame,conformance)* expand frame operations and conformance harness
- complete essence extraction for FP-P2C-006..011 and expand columnar/frame/groupby implementations
- *(frame)* add Series.align(), combine_first(), reindex() with join mode support
- series arithmetic, constructors, join/concat, and alien artifact enhancements
- expand groupby aggregation, join operations, and conformance testing

### <!-- 1 -->Fixed

- *(fp-frame)* match pandas mode dropna dtype
- *(fp-frame)* match pandas mode padding dtype
- *(frame)* sort unique Series arithmetic union (frankenpandas-cod1d13)
- *(frame)* implement unicode string normalization (frankenpandas-cod2-7354bc8c)
- *(fp-frame)* preserve column_order on serde round-trip (fd90.196 / br-1d1gm)
- *(fp-frame)* honor categorical ordering semantics (frankenpandas-8qdg)
- *(dtype)* wire sparse marker through crate policies (frankenpandas-0xcm)
- *(frame)* align rolling count null windows
- *(join)* align outer join concat parity
- *(io)* align csv and json parity
- *(datetime)* coerce mixed timezone to_datetime
- *(api)* gate all pub error enums with #[non_exhaustive] (br-frankenpandas-tne4)
- *(fp-frame)* restore grouped window fallback schema (br-frankenpandas-ch4o)
- *(fp-frame)* preserve Int64/Bool dtype in DataFrame.mode pad cells (br-frankenpandas-gwa2)
- *(fp-frame)* reject table json for MultiIndex columns (br-frankenpandas-qwye)
- *(fp-frame)* Expanding mean/min/max empty-window NaN parity (frankenpandas-3kew)
- *(fp-frame)* Series.unique retains a single null marker (frankenpandas-378p)
- *(fp-frame)* Series.searchsorted accepts non-numeric needles (frankenpandas-yrhm)
- *(fp-frame)* unblock tight MultiIndex column metadata test (frankenpandas-7jzs)
- *(fp-frame)* to_dict orient='index' rejects duplicate index (frankenpandas-sewx)
- *(fp-frame)* rolling min min_periods=0 parity (frankenpandas-brti)
- *(fp-frame)* reject empty separator in str.split_expand
- *(str)* column-aware expandtabs + cased-run based title() semantics
- *(hygiene)* apply rustfmt to fp-frame and fp-join
- integer precision in columnar ops, N-way concat alignment, and merge_asof direction
- *(fp-frame)* correct filter/loc_slice alignment and handle empty Series/DataFrame
- *(docs)* fix misaligned right-side borders in all 3 ASCII art diagrams
- *(fp-frame)* agg_named rejects duplicate output names, fix test assertions
- *(fp-frame)* fix negative timedelta round-trip bug in parse_hms
- *(fp-frame)* to_multi_index now handles Float64/Bool columns correctly
- *(fp-frame)* use explicit dereference in max_by_key closure
- *(fp-frame)* resolve clippy warnings in interpolate_method and pivot_table_with_margins
- *(fp-frame)* use explicit Scalar constructors in isin_dict and applymap tests
- *(frame)* format bool groupby key labels as pandas-style "True"/"False"
- *(test)* use exactly-representable float in JSON round-trip test
- *(fp-frame)* filter non-numeric columns in corr/cov pairwise_stat
- *(frame)* refine DataFrame operations for correctness and code clarity
- *(fp-frame)* use Column::from_values in applymap and fix groupby count test
- *(frame)* add Bool variant to debug column name parser and use type-inferred Column construction
- *(frame)* remove dead branches in pct_change and rolling_count

### <!-- 3 -->Changed

- *(fp-frame)* remove dead DuplicateIndexUnsupported variant (br-yvkrb / fd90.145)
- *(fp-frame)* convert where_mask_df_other to delegator over where_cond_df (br-a2sck / fd90.139)
- *(fp-frame)* replace implicit .into() with explicit Scalar::Int64() in test index labels

### <!-- 4 -->Documentation

- *(README)* refresh DISC-013 status now that it lives in Resolved
- *(readme)* fix divergence count - 14 documented, 4 cause failures
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
- *(fp-frame)* add crate-level rustdoc (br-9s0g / fd90.65)
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

- *(frame)* freeze display goldens (frankenpandas-cod1m03)
- *(frankenpandas)* add Missing Data Handling test + fix README dropna_with_threshold arity (br-jl1go / fd90.172)
- *(conformance)* add io formats to_html matrix
- *(conformance)* add MultiIndex parity matrix (br-frankenpandas-m8cp)
- *(fuzz)* add rolling window min_periods target (br-frankenpandas-6xub)
- *(fp-frame)* pin grouped resample sum/mean dtype+null semantics (i2j8)
- *(fp-frame)* grouped resample first/last 3-group + sparse coverage (br-frankenpandas-icm3)
- *(fp-frame)* align grouped window output with pandas (br-frankenpandas-r4au)
- *(fp-frame)* grouped rolling min/max/count edge-case coverage (br-frankenpandas-4hkt)
- *(fp-frame)* grouped resample first/last edge-case coverage (br-frankenpandas-auwr)
- *(fp-frame)* grouped rolling std/var Int64 + all-NaN coverage (br-frankenpandas-9uuq)
- lock pivot_table median margins parity (br-i77n)
- *(to_datetime)* add timestamp-like origin parity (frankenpandas-oxh1)
- *(fp-frame)* add drop_duplicates subset order parity (frankenpandas-r0c1)
- *(fp-frame)* add unicode removeprefix exactness parity (frankenpandas-88mp)
- add Series.between left/right parity packets (frankenpandas-81cv)
- *(frame)* add comprehensive from_csv_with_options test coverage

### <!-- 9 -->Reverted

- Revert "refactor(fp-frame): replace implicit .into() with explicit Scalar::Int64() in test index labels"

### Bd-1q3z

- add FP-P2D-030 axis0 inner concat parity

### Bd-z45l

- add FP-P2D-029 dataframe concat axis=1 inner parity

### Br-frankenpandas-a7v2

- add grouped resample first last conformance test

### Br-frankenpandas-khv6

- add grouped resample mean conformance test

### Br-frankenpandas-oguf

- add grouped resample min max conformance test

### Br-frankenpandas-p6vu

- add grouped resample count conformance test

### Br-frankenpandas-x5oy

- add grouped rolling conformance coverage

### Conformance

- broaden dataframe merge/constructor parity surface and packet coverage

### Fp-expr/fp-frame

- add boolean mask composition operators and planner logical ops (bd-9ksz)

### Fp-frame

- add to_json "table" orient with Table Schema metadata
- add to_dict("series") orient returning Series-typed values
- extend DataFrame::align_on_index to handle duplicate labels
- add multi-column astype mapping parity (bd-1ldj)
- add DataFrame scalar-broadcast constructor parity (bd-22v1)
- add duplicated/drop_duplicates DataFrame parity (bd-uket)
- port DataFrame sort_index/sort_values core APIs (bd-1zco)

### Licensing

- adopt MIT + OpenAI/Anthropic rider across workspace and crates

### Port

- validate malformed Series.dt.to_timestamp periods (frankenpandas-16hq)

### Style

- apply rustfmt cleanup after frankenpandas-16hq
- *(fp-columnar,fp-frame,fp-io)* clippy and rustfmt cleanup
- *(frame)* normalize rustfmt formatting in from_csv_with_options
- normalize rustfmt formatting across fp-expr, fp-frame, fp-groupby, fp-io, fp-join
