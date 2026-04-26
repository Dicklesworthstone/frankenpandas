# FEATURE_PARITY

## Status Legend

- not_started
- in_progress
- parity_green
- parity_gap

## Parity Matrix

| Feature Family | Status | Notes |
|---|---|---|
| DataFrame/Series constructors | in_progress | `Series::from_values` + `DataFrame::from_series` MVP implemented; DataFrame constructor scalar-broadcast parity now available via `from_dict_mixed`/`from_dict_with_index_mixed` (fail-closed all-scalar-without-index and mismatched-shape guards); `FP-P2C-003` extends arithmetic fixture coverage; DataFrame `iterrows`/`itertuples`/`items`/`assign`/`pipe` + `where_cond`/`mask` implemented; broader constructor parity pending |
| Expression planner arithmetic | in_progress | `fp-expr` now supports `Expr::Add`/`Sub`/`Mul`/`Div`, logical mask composition (`Expr::And`/`Or`/`Not`), plus `Expr::Compare` (`eq`/`ne`/`lt`/`le`/`gt`/`ge`) with full-eval and incremental-delta paths (including series-scalar anchoring); includes DataFrame-backed eval/query bridges (`EvalContext::from_dataframe`, `evaluate_on_dataframe`, `filter_dataframe_on_expr`); string expression parser with `parse_expr`/`eval_str`/`query_str` for pandas-style `df.eval("a + b")` and `df.query("x > 5 and y < 10")` semantics; broader window/string/date expression surface still pending |
| Index alignment and selection | in_progress | `FP-P2C-001`/`FP-P2C-002` packet suites green with gate validation and RaptorQ sidecars; `FP-P2C-010` adds series/dataframe filter-head-loc-iloc basics plus live-oracle coverage for `Series::take`, `Series::at_time`, and `Series::between_time`; `FP-P2D-025` extends DataFrame loc/iloc row+column selector parity; `FP-P2D-026` adds DataFrame head/tail parity; `FP-P2D-027` adds DataFrame head/tail negative-`n` parity; `FP-P2D-040` adds DataFrame `sort_index`/`sort_index_axis1`/`sort_values` ordering parity (including descending and NA-last cases); `FP-P2D-067` now also covers live-oracle parity for `DataFrame::take`, `DataFrame::asof(where, subset)`, `DataFrame::at_time`, and `DataFrame::between_time`; `fp-frame` now also exposes Series `head`/`tail` including negative-`n` semantics and time-of-day selectors `at_time`/`between_time`, DataFrame row lookup `asof(where, subset)`, time-of-day selectors `at_time`/`between_time`, `set_index`/`reset_index` (single-index model, including mixed Int64/Utf8 index-label reset materialization), row-level `duplicated`/`drop_duplicates`, and `DataFrame::drop(labels, axis)` for flexible row/column removal plus `drop_rows_int`; broader selector+ordering matrix still pending |
| Series conditional/membership | in_progress | `where_cond`/`mask`/`isin`/`between` implemented with tests; broader conditional matrix pending |
| Series statistics (extended) | in_progress | `idxmin`/`idxmax`/`nlargest`/`nsmallest`/`pct_change`/`corr`/`cov_with`/`prod`/`mode` implemented; `Series::dtype()` accessor added; `dt` accessor with `year`/`month`/`day`/`hour`/`minute`/`second`/`dayofweek`/`date`/`quarter`/`dayofyear`/`weekofyear`/`is_month_start`/`is_month_end`/`is_quarter_start`/`is_quarter_end`/`strftime`; `map_fn` failable closure mapping; binary ops with fill_value: `add_fill`/`sub_fill`/`mul_fill`/`div_fill`; `modulo`/`pow` element-wise ops; `nlargest_keep`/`nsmallest_keep` with keep param ('first'/'last'/'all'); positional indexing: `argsort`/`argmin`/`argmax`/`take`/`searchsorted`/`factorize`, including pandas-style negative indices for `take`; higher-order stats: `sem`/`skew`/`kurtosis`/`kurt`; `squeeze`/`memory_usage` introspection; `append` for concatenation; `autocorr(lag)` autocorrelation; `dot(other)` inner product; broader stats pending |
| Series str accessor | in_progress | `StringAccessor` with `lower`/`upper`/`strip`/`lstrip`/`rstrip`/`contains`/`replace`/`startswith`/`endswith`/`len`/`slice`/`split_get`/`split_df`/`rsplit_get`/`rsplit_df`/`capitalize`/`title`/`repeat`/`pad` plus regex methods: `contains_regex`/`replace_regex`/`replace_regex_all`/`extract`/`extract_df`/`extractall`/`count_matches`/`findall`/`fullmatch`/`match_regex`/`split_regex_get`; `extract_df`, `extractall`, `partition_df`, and `rpartition_df` now have live-oracle conformance coverage; formatting: `zfill`/`center`/`ljust`/`rjust`; predicates: `isdigit`/`isalpha`/`isalnum`/`isspace`/`islower`/`isupper`/`isnumeric`/`isdecimal`/`istitle`; additional: `get`/`wrap`/`normalize`/`cat`/`split_count`/`join`; search: `find`/`rfind`/`index_of`/`rindex_of`; case: `casefold`/`swapcase`; split: `partition`/`partition_df`/`rpartition`/`rpartition_df`; prefix/suffix: `removeprefix`/`removesuffix`; whitespace: `expandtabs`; `get_dummies(sep)` string-split to indicator DataFrame; broader str methods pending |
| DataFrame groupby integration | in_progress | `DataFrame::groupby(&[columns])` returns `DataFrameGroupBy` with `sum`/`mean`/`count`/`min`/`max`/`std`/`var`/`median`/`first`/`last`/`size`/`nunique`/`prod` aggregation; multi-column group keys with composite key support; `agg()` per-column mapping and `agg_list()` multi-function; `apply()` custom closure; `transform()` shape-preserving broadcast; `filter()` group predicate; cumulative transforms: `cumsum`/`cumprod`/`cummax`/`cummin`; within-group ops: `rank`/`shift`/`diff`/`nth`/`head`/`tail`/`pct_change`/`ffill`/`bfill`/`value_counts`/`describe`; group selection: `get_group(name)`/`cumcount()`/`ngroup()`/`pipe()` now has live-oracle conformance coverage for `get_group(name)`, `cumcount()`, `ngroup()`, `ffill()`, and `bfill()`; statistics: `sem()`/`skew()`/`kurtosis()` now also have live-oracle conformance coverage; time-series: `ohlc()` now has live-oracle conformance coverage for the current flat-column contract (single-value `open/high/low/close`, multi-value `<column>_<stat>`), while full pandas MultiIndex-column parity remains pending; broader custom function patterns pending |
| DataFrame properties/introspection | in_progress | `shape()`, `dtypes()`, `copy()`, `keys()` info-axis alias, `to_dict(orient)` with dict/list/records/index/split orients; `info()` string summary; `sample(n, frac, replace, seed)` with deterministic LCG + Fisher-Yates; `compare(other)` element-wise diff; `squeeze(axis)`/`squeeze_to_series(axis)` single-column/row reduction; `bool_()` single-element boolean extraction with pandas-style boolean-only semantics; `memory_usage()` per-column byte estimates; `ndim()` (always 2), `axes()` (index + columns), `empty`/`is_empty()` checks; `equals(other)` deep comparison; `first_valid_index()`/`last_valid_index()` null scanning; `isin(values)` element membership; `mode()` per column; `corrwith(other)` column-wise correlation; `dot(other)` matrix multiplication; `lookup(rows, cols)` label-based element access; `t()`/`swapaxes()` transpose aliases |
| Series conversion | in_progress | `to_frame`/`to_list`/`to_dict`/`to_csv`/`to_json(split/records/index)`/`explode(sep)` implemented; `from_dict`/`from_pairs` dict constructors; `to_string_repr()` pretty-print; `value_counts_with_options(normalize, sort, ascending, dropna)` full-param variant; `is_unique`/`is_monotonic_increasing`/`is_monotonic_decreasing` introspection; `hasnans`/`nbytes`/`pipe`/`items` utility methods; `keys()` index alias; `iat(pos)`/`at(label)` scalar accessors; `bool_()` single-element boolean extraction with pandas-style boolean-only semantics; `transform(func)` element-wise; `rename(name)`/`set_axis(labels)` label ops; `truncate(before, after)` index slicing; `combine(other, func)` and `combine_first(other)` combination helpers, with differential packet + live-oracle coverage for object-like fill-precedence/union semantics; `sample(n, frac, replace, seed)` random sampling; `repeat(repeats)` with scalar and per-element repeat counts; `duplicated()`/`drop_duplicates()` duplicate handling; `compare_with(other)` diff format; `reindex_like(other)` index matching; `convert_dtypes()` type inference (Utf8→Int64/Float64); `map_with_na_action(mapping, na_action_ignore)` NaN-skipping map; `case_when(cases)` conditional value assignment; `drop(labels)` index-label removal; `asof(label)` last non-NaN at/before label; `unstack()` composite-index to DataFrame |
| Series/DataFrame rank | in_progress | `rank()` with `average`/`min`/`max`/`first`/`dense` methods, `ascending`/`descending`, `na_option` keep/top/bottom, plus DataFrame `axis=1` row-wise ranking; full edge-case matrix pending |
| Rolling/Expanding/EWM windows | in_progress | `Series::rolling(window).sum/mean/min/max/std/count/var/median/quantile/apply()` and `Series::expanding().sum/mean/min/max/std/var/median/apply()` plus `DataFrame::rolling(window).sum/mean/min/max/std/count/var/median/quantile()` and `DataFrame::expanding().sum/mean/min/max/std/var/median()` implemented; `Series::ewm(span/alpha).mean/std/var()` exponentially weighted moving window; `Series::resample(freq).sum/mean/count/min/max/first/last()` time-based resampling; broader window ops pending |
| DataFrame/Series reshaping | in_progress | `melt(id_vars, value_vars, var_name, value_name)` and `pivot_table(values, index, columns, aggfunc)` implemented with sum/mean/count/min/max/first; `pivot(index, columns, values)` simple non-aggregation reshape; `stack`/`unstack` implemented with composite key round-trip; `crosstab(index, columns)` contingency tables with `crosstab_normalize(all/index/columns)` normalization; `explode(column, sep)` unnest string columns; `xs(key)` cross-section selection (Series and DataFrame), now with duplicate-label live-oracle conformance coverage; `droplevel()` index reset (Series and DataFrame); broader reshaping edge cases pending |
| DataFrame aggregation | in_progress | `agg()` with per-column named functions, `applymap()`/`map_elements()` for element-wise ops, `transform()` shape-preserving variant; column-wise (axis=0): `sum`/`mean`/`min_agg`/`max_agg`/`std_agg`/`var_agg`/`median_agg`/`prod_agg`/`skew_agg`/`kurtosis_agg`/`sem_agg`/`count`/`nunique`/`idxmin`/`idxmax`/`all`/`any`; row-wise (axis=1): `sum_axis1`/`mean_axis1`/`min_axis1`/`max_axis1`/`std_axis1`/`var_axis1`/`median_axis1`/`prod_axis1`/`count_axis1`/`nunique_axis1`/`all_axis1`/`any_axis1`; `apply_row`/`apply_row_fn` for row-wise closures returning Series; broader aggregation patterns pending |
| DataFrame correlation/covariance | in_progress | `corr()`/`cov()` pairwise matrices with Pearson method; `corr_method("spearman")`/`corr_method("kendall")` now implemented including `corr_spearman()`/`corr_kendall()` on Series; broader rank correlation edge cases pending |
| DataFrame element-wise ops | in_progress | `cumsum`/`cumprod`/`cummax`/`cummin`/`diff`/`shift`/`abs`/`clip`/`clip_lower`/`clip_upper`/`round`/`pct_change`/`replace` implemented for DataFrame, including `shift(axis=1)` horizontal column shifts; `combine_first(other)` now has differential packet + live-oracle coverage for object-like fill-precedence/union semantics; `ffill`/`bfill`/`interpolate` per-column fill methods; `to_csv(sep, include_index)`/`to_json(orient)`/`to_string_table(include_index)`/`to_markdown(include_index)`/`to_latex(include_index)`/`to_html(include_index)` string export; `from_csv(text, sep)` CSV parsing; `rename_with(func)`/`add_prefix`/`add_suffix` column renaming; `append(other)` row concatenation; `add_scalar`/`sub_scalar`/`mul_scalar`/`div_scalar`/`pow_scalar`/`mod_scalar`/`floordiv_scalar` scalar arithmetic; `add_df`/`sub_df`/`mul_df`/`div_df`/`floordiv_df`/`mod_df` DataFrame-to-DataFrame arithmetic with index alignment; `assign_column(name, values)` set/replace column; `convert_dtypes()` type inference; delegates to per-column Series methods, non-numeric columns preserved |
| DataFrame comparison ops | in_progress | `eq_df`/`ne_df`/`gt_df`/`ge_df`/`lt_df`/`le_df` element-wise DataFrame-to-DataFrame comparison returning Bool DataFrames with index alignment; `eq_scalar_df`/`ne_scalar_df`/`gt_scalar_df`/`ge_scalar_df`/`lt_scalar_df`/`le_scalar_df` scalar broadcast comparison; `compare_scalar_df(scalar, op)` generic scalar comparison; NaN semantics: NaN eq anything = false, NaN ne anything = true |
| DataFrame reshaping (extended) | in_progress | `get_dummies(columns)` one-hot encoding with auto-Utf8 detection, multi-column support, discovery-order indicator columns; existing: `melt`/`pivot_table`/`pivot`/`stack`/`unstack`/`crosstab`/`explode`/`xs`/`droplevel` |
| Module-level utility functions | in_progress | `isna`/`isnull`/`notna`/`notnull` module-level null checks; `to_numeric(series)` coerce-to-numeric with NaN for non-parseable; `cut(series, bins)` equal-width binning; `qcut(series, q)` quantile-based binning; live-oracle conformance coverage now exercises `to_numeric`/`cut`/`qcut`; existing: `concat_series`/`concat_dataframes`/`concat_dataframes_with_axis`/`concat_dataframes_with_axis_join`/`concat_dataframes_with_keys` |
| DataFrame selection (extended) | in_progress | `nlargest(n, column)`/`nsmallest(n, column)` for top-N rows + `nlargest_keep`/`nsmallest_keep` with first/last/all keep param; `reindex()` for label-based reindexing; `value_counts_per_column()` and `value_counts()` (unique row counting); `insert(loc, name, column)` positional column insertion; `pop(name)` remove-and-return column; `align_on_index(other, mode)` DataFrame index alignment; `select_dtypes(include, exclude)` dtype-based column selection; `filter_labels(items, like, regex, axis)` flexible row/column filtering; `fillna_method(method)` for 'ffill'/'bfill' fill; `get_column(key)` with NaN default; `bool_()` single-element extraction; `where_mask_df(cond_df, other)`/`mask_df(cond_df, other)` DataFrame-condition masking; `take(indices, axis)` positional row/column selection with negative-index support, plus `take_rows(indices)`/`take_columns(indices)` normalized helpers; `infer_objects()` dtype inference |
| DataFrame constructors (extended) | in_progress | `from_dict_index(data)` orient='index' constructor; `from_dict_index_columns(data, columns)` orient='index' with column names; existing: `from_dict`, `from_dict_mixed`, `from_dict_with_index`, `from_dict_with_index_mixed`, `from_records`, `from_series`, `from_csv` |
| GroupBy core aggregates | in_progress | `FP-P2C-005` and `FP-P2C-011` suites green (`sum`/`mean`/`count` core semantics); `nunique`/`prod`/`size`/`idxmin`/`idxmax`/`quantile`/`any`/`all` added for DataFrameGroupBy, with live-oracle conformance coverage for grouped `idxmin()`/`idxmax()`/`any()`/`all()`; `agg()` per-column and `agg_list()` multi-func; `apply()`/`transform()`/`filter()` implemented; broader aggregate matrix still pending |
| Join/merge/concat core | in_progress | `FP-P2C-004` and `FP-P2C-006` suites green for series-level join/concat semantics; `FP-P2D-014` covers DataFrame merge + axis=0 concat matrix; `FP-P2D-028` adds DataFrame concat axis=1 outer alignment parity; `FP-P2D-029` adds axis=1 `join=inner` parity; `FP-P2D-030` adds axis=0 `join=inner` shared-column parity; `FP-P2D-031` adds axis=0 `join=outer` union-column/null-fill parity; `FP-P2D-032` adds axis=0 `join=outer` first-seen column-order (`sort=False`) parity; `FP-P2D-039` adds DataFrame merge `how='cross'` semantics; `DataFrameMergeExt` trait in fp-join adds `merge()`/`merge_with_options()`/`join_on_index()`/`merge_asof()`/`merge_ordered()` instance methods; `merge_asof(left, right, on, direction)` for nearest-match time-series joins (backward/forward/nearest); `merge_ordered(left, right, on, fill_method)` for ordered merges with optional fill and live-oracle conformance coverage; `concat_dataframes_with_keys(frames, keys)` for hierarchical index labeling; full DataFrame merge/concat contracts still pending |
| Null/NaN semantics | in_progress | `FP-P2C-007` suite green for `dropna`/`fillna`/`nansum`; `fp-frame` now also exposes Series/DataFrame `isna`/`notna` plus `isnull`/`notnull` aliases, DataFrame `fillna`, optioned row-wise `dropna` (`how='any'/'all'` + `thresh` with column `subset` selectors), and optioned column-wise `dropna` (`axis=1`, `how='any'/'all'` + `thresh`, row-label `subset`, plus default `dropna_columns()`); Series/DataFrame `ffill`/`bfill` with optional limit and `interpolate()` linear; full nanops matrix still pending |
| Core CSV ingest/export | in_progress | `FP-P2C-008` suite green for CSV round-trip core cases; `fp-io` now supports optioned file-based CSV reads (`read_csv_with_options_path`) and JSON `records`/`columns`/`split`/`index` orients (including split index-label roundtrip); broader parser/formatter parity matrix pending |
| Parquet I/O | in_progress | `fp-io` supports `read_parquet`/`write_parquet` (file-based) and `read_parquet_bytes`/`write_parquet_bytes` (in-memory) via Arrow RecordBatch integration; handles Int64/Float64/Bool/Utf8 dtypes with null round-trip; multi-batch reading via `concat_dataframes`; `DataFrameIoExt` extension trait adds `to_parquet()`/`to_parquet_bytes()`/`to_csv_file()`/`to_json_file()`/`to_excel_file()`/`to_excel_bytes()` convenience methods on DataFrame; broader Parquet options (compression, row-group control, predicate pushdown) pending |
| Excel (xlsx) I/O | in_progress | `fp-io` supports `read_excel`/`write_excel` (file-based) and `read_excel_bytes`/`write_excel_bytes` (in-memory) via `calamine` (read) and `rust_xlsxwriter` (write); `ExcelReadOptions` with `sheet_name`, `has_headers`, `index_col`, `skip_rows`; Int64/Float64/Bool/Utf8 dtypes with null round-trip; integer recovery from Excel f64 for whole numbers; multi-format support (.xlsx/.xls/.xlsb/.ods via calamine); broader Excel options (formatting, multiple sheets, date handling) pending |
| JSONL (JSON Lines) I/O | in_progress | `fp-io` supports `write_jsonl_string`/`read_jsonl_str` (in-memory) and `write_jsonl`/`read_jsonl` (file-based); matches `pd.to_json(lines=True)` and `pd.read_json(lines=True)`; one JSON object per line, no array wrapper; blank lines skipped on read; `DataFrameIoExt` trait adds `to_jsonl_file()` |
| Arrow IPC / Feather I/O | in_progress | `fp-io` supports `write_feather`/`read_feather` (file-based) and `write_feather_bytes`/`read_feather_bytes` (in-memory) via Arrow IPC file format; `write_ipc_stream_bytes`/`read_ipc_stream_bytes` for streaming IPC format; `DataFrameIoExt` trait adds `to_feather_file()`/`to_feather_bytes()`; reuses existing RecordBatch conversion infrastructure; Int64/Float64/Bool/Utf8 dtypes with null round-trip; multi-batch reading via concat_dataframes; broader options (compression, column selection) pending |
| SQL (SQLite) I/O | in_progress | `fp-io` supports `read_sql(conn, query)`/`read_sql_table(conn, table_name)`/`write_sql(frame, conn, table_name, if_exists)` via `rusqlite` with bundled SQLite; `SqlIfExists` enum (Fail/Replace/Append); `DataFrameIoExt` trait adds `to_sql()` convenience method; Int64<->INTEGER, Float64<->REAL, Utf8<->TEXT, Bool->INTEGER(0/1), Null<->NULL dtype mapping; transaction-wrapped bulk inserts; SQL injection prevention via table name validation; broader SQL options (schema, dtype overrides, chunksize, multi-DB via sqlx) pending |
| MultiIndex foundation | in_progress | `MultiIndex` struct in fp-index with `levels` (Vec<Vec<IndexLabel>>) + `names`; constructors: `from_tuples`/`from_arrays`/`from_product` (Cartesian); accessors: `nlevels()`/`len()`/`names()`/`get_level_values(level)`/`get_tuple(pos)`; operations: `to_flat_index(sep)`/`droplevel(level)` (returns `MultiIndexOrIndex`)/`swaplevel(i,j)`/`set_names()`; additive type (existing Index unchanged); broader DataFrame integration (MultiIndex as row index, groupby MultiIndex output, stack/unstack) pending |
| Index utility methods | in_progress | `min()`/`max()`/`argmin()`/`argmax()` aggregation; `nunique()` unique count; `map(func)`/`rename(func)` label transformation; `drop_labels(labels)` removal; `astype_int()`/`astype_str()` type conversion; `fillna()`/`isna()`/`notna()` null handling (no-op for non-nullable labels); `where_cond(cond, other)` conditional replacement; existing: `is_unique`/`get_loc`/`get_indexer`/`isin`/`contains`/`unique`/`duplicated`/`drop_duplicates`/`sort_values`/`argsort`/`take`/`slice`/`from_range` |
| Categorical data | in_progress | `CategoricalMetadata` struct with categories + ordered flag stored as optional metadata on `Series`; codes stored as Int64 column; `Series::from_categorical(values, ordered)` and `Series::from_categorical_codes(codes, categories, ordered)` constructors; `.cat()` accessor with `CategoricalAccessor` providing `categories()`/`codes()`/`ordered()`/`rename_categories()`/`add_categories()`/`remove_unused_categories()`/`set_categories()`/`as_ordered()`/`as_unordered()`/`to_values()`; implemented as metadata layer (no DType enum change); broader integration with groupby ordering, value_counts category awareness, and dtype reporting pending |
| pd.to_datetime() | in_progress | `to_datetime(series)`, `to_datetime_with_format(series, format)`, `to_datetime_with_unit(series, unit)`, and `to_datetime_with_options(series, ToDatetimeOptions)` module-level functions; auto-detects ISO 8601 (date/datetime/T/space), slash dates (YYYY/MM/DD), US dates (MM/DD/YYYY), and fixed-offset/`Z` timezone-aware strings with normalized output; epoch seconds/milliseconds (auto-detected from magnitude); explicit `unit=` conversions for `D`/`s`/`ms`/`us`/`ns` with subsecond precision preservation; `utc=true` localizes naive values and converts fixed-offset timezone-aware values to UTC; `origin=` now supports Unix, Julian-day (`origin='julian'` with `unit='D'`), numeric offsets relative to the Unix epoch, and custom timestamp/date origins; custom format strings via chrono strftime; missing/unparseable → NaT; outputs normalized ISO 8601 Utf8 strings; broader pandas DST-heavy mixed-timezone parsing and timestamp-like object `origin` variants pending |
| Timedelta type | in_progress | `DType::Timedelta64` and `Scalar::Timedelta64(i64)` with nanosecond precision storage; `Timedelta` helper struct with `NANOS_PER_*` constants and `NAT` sentinel (i64::MIN); string parsing for compound durations (`1d 2h 30m`), simple units (`1d`, `2h`, `30m`, `45s`, `100ms`), and time format (`01:30:00`); `Timedelta::format()` for display; `Timedelta::components()` for days/hours/minutes/seconds/milliseconds/microseconds/nanoseconds extraction; `Timedelta::total_seconds()` for f64 conversion; `IndexLabel::Timedelta64` for TimedeltaIndex with `Index::from_timedelta64()` and `timedelta_range(start, end, periods, freq, name)`; `to_timedelta(series)`, `to_timedelta_with_unit(series, unit)`, and `to_timedelta_with_options(series, ToTimedeltaOptions)` module-level functions with unit parsing (D/h/m/s/ms/us/ns/W) and `errors='raise'/'coerce'/'ignore'` semantics; Series Timedelta arithmetic: `td_add`/`td_sub`/`td_mul_scalar`/`td_div_scalar`/`td_floordiv_scalar`/`td_ratio`/`td_mod`/`td_neg`/`td_abs` plus comparison ops `td_lt`/`td_le`/`td_gt`/`td_ge`/`td_eq`/`td_ne` with NaT propagation; full integration with fp-columnar, fp-frame, fp-io, fp-groupby, fp-conformance, fp-join; broader Timedelta-Datetime interop pending |
| Expression evaluation | in_progress | `fp-expr` provides `eval_str()`/`query_str()` for expression parsing and evaluation; `DataFrameExprExt` extension trait adds `df.eval(expr)`/`df.query(expr)` convenience methods on DataFrame; supports arithmetic (`+`/`-`/`*`/`/`), comparison (`>`/`<`/`==`/`!=`/`>=`/`<=`), logical (`and`/`or`/`not`), column references, and scalar literals |
| Storage/dtype invariants | in_progress | `FP-P2C-009` suite green for dtype invariant checks; `fp-frame` now exposes `Series::astype` plus DataFrame single- and multi-column coercion via `astype_column` and mapping-based `astype_columns`; broader dtype coercion/storage matrix pending |

## Phase-2C Packet Evidence (Foundational subset)

> **Note:** The table below shows the FOUNDATIONAL subset of packet suites
> (the originally enumerated parity-green checkpoints from Phase 2C/2D
> ramp-up). The live packet ledger has expanded substantially since:
> as of this commit, **430 distinct packet suites** exist across
> `crates/fp-conformance/fixtures/packets/` (11 FP-P2C + 419 FP-P2D,
> ranging FP-P2D-014 through FP-P2D-433+). For the full real-time view,
> see `artifacts/phase2c/drift_history.jsonl` and the per-packet
> directories under `artifacts/phase2c/`. The README's Testing section
> tracks the running pass/fail count.

| Packet | Result | Evidence |
|---|---|---|
| FP-P2C-001 | parity_green | `artifacts/phase2c/FP-P2C-001/parity_report.json`, `artifacts/phase2c/FP-P2C-001/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-001/parity_report.raptorq.json` |
| FP-P2C-002 | parity_green | `artifacts/phase2c/FP-P2C-002/parity_report.json`, `artifacts/phase2c/FP-P2C-002/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-002/parity_report.raptorq.json` |
| FP-P2C-003 | parity_green | `artifacts/phase2c/FP-P2C-003/parity_report.json`, `artifacts/phase2c/FP-P2C-003/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-003/parity_report.raptorq.json` |
| FP-P2C-004 | parity_green | `artifacts/phase2c/FP-P2C-004/parity_report.json`, `artifacts/phase2c/FP-P2C-004/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-004/parity_report.raptorq.json` |
| FP-P2C-005 | parity_green | `artifacts/phase2c/FP-P2C-005/parity_report.json`, `artifacts/phase2c/FP-P2C-005/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-005/parity_report.raptorq.json` |
| FP-P2C-006 | parity_green | `artifacts/phase2c/FP-P2C-006/parity_report.json`, `artifacts/phase2c/FP-P2C-006/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-006/parity_report.raptorq.json` |
| FP-P2C-007 | parity_green | `artifacts/phase2c/FP-P2C-007/parity_report.json`, `artifacts/phase2c/FP-P2C-007/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-007/parity_report.raptorq.json` |
| FP-P2C-008 | parity_green | `artifacts/phase2c/FP-P2C-008/parity_report.json`, `artifacts/phase2c/FP-P2C-008/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-008/parity_report.raptorq.json` |
| FP-P2C-009 | parity_green | `artifacts/phase2c/FP-P2C-009/parity_report.json`, `artifacts/phase2c/FP-P2C-009/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-009/parity_report.raptorq.json` |
| FP-P2C-010 | parity_green | `artifacts/phase2c/FP-P2C-010/parity_report.json`, `artifacts/phase2c/FP-P2C-010/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-010/parity_report.raptorq.json` |
| FP-P2C-011 | parity_green | `artifacts/phase2c/FP-P2C-011/parity_report.json`, `artifacts/phase2c/FP-P2C-011/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-011/parity_report.raptorq.json` |
| FP-P2D-025 | parity_green | `artifacts/phase2c/FP-P2D-025/parity_report.json`, `artifacts/phase2c/FP-P2D-025/parity_gate_result.json`, `artifacts/phase2c/FP-P2D-025/parity_report.raptorq.json` |
| FP-P2D-026 | parity_green | `artifacts/phase2c/FP-P2D-026/parity_report.json`, `artifacts/phase2c/FP-P2D-026/parity_gate_result.json`, `artifacts/phase2c/FP-P2D-026/parity_report.raptorq.json` |
| FP-P2D-027 | parity_green | `artifacts/phase2c/FP-P2D-027/parity_report.json`, `artifacts/phase2c/FP-P2D-027/parity_gate_result.json`, `artifacts/phase2c/FP-P2D-027/parity_report.raptorq.json` |
| FP-P2D-028 | parity_green | `artifacts/phase2c/FP-P2D-028/parity_report.json`, `artifacts/phase2c/FP-P2D-028/parity_gate_result.json`, `artifacts/phase2c/FP-P2D-028/parity_report.raptorq.json` |
| FP-P2D-029 | parity_green | `artifacts/phase2c/FP-P2D-029/parity_report.json`, `artifacts/phase2c/FP-P2D-029/parity_gate_result.json`, `artifacts/phase2c/FP-P2D-029/parity_report.raptorq.json` |
| FP-P2D-030 | parity_green | `artifacts/phase2c/FP-P2D-030/parity_report.json`, `artifacts/phase2c/FP-P2D-030/parity_gate_result.json`, `artifacts/phase2c/FP-P2D-030/parity_report.raptorq.json` |
| FP-P2D-031 | parity_green | `artifacts/phase2c/FP-P2D-031/parity_report.json`, `artifacts/phase2c/FP-P2D-031/parity_gate_result.json`, `artifacts/phase2c/FP-P2D-031/parity_report.raptorq.json` |
| FP-P2D-032 | parity_green | `artifacts/phase2c/FP-P2D-032/parity_report.json`, `artifacts/phase2c/FP-P2D-032/parity_gate_result.json`, `artifacts/phase2c/FP-P2D-032/parity_report.raptorq.json` |
| FP-P2D-039 | parity_green | `artifacts/phase2c/FP-P2D-039/parity_report.json`, `artifacts/phase2c/FP-P2D-039/parity_gate_result.json`, `artifacts/phase2c/FP-P2D-039/parity_report.raptorq.json` |
| FP-P2D-040 | parity_green | `artifacts/phase2c/FP-P2D-040/parity_report.json`, `artifacts/phase2c/FP-P2D-040/parity_gate_result.json`, `artifacts/phase2c/FP-P2D-040/parity_report.raptorq.json` |

Gate enforcement and trend history:

- blocking command: `./scripts/phase2c_gate_check.sh`
- CI workflow: `.github/workflows/ci.yml`
- drift history ledger: `artifacts/phase2c/drift_history.jsonl`

## Required Evidence Per Feature Family

1. Differential fixture report.
2. Edge-case/adversarial test results.
3. Benchmark delta (when performance-sensitive).
4. Documented compatibility exceptions (if any).

<!-- BEGIN AUTO-PACKET-TABLE -->
<!-- Auto-generated by scripts/gen_feature_parity_table.py — do not edit by hand. -->

> **What `pass` means here:** the `pass` boolean inside
> `parity_gate_result.json` reflects the per-packet **gate threshold**
> (typically: 0 fixture failures required). It is a stricter signal
> than the lib-level `cargo test -p fp-conformance --lib` pass count
> (~557/562 = 99% as of the README's Testing section), which counts
> individual `#[test]` functions returning `Ok`. A packet can have
> hundreds of green lib tests but still report `pass: false` here if
> any single fixture inside it tripped the per-packet gate. Read
> aggregate counts below as gate-level, not lib-test-level.

**Aggregate status (live from `artifacts/phase2c/`):**

- Total packet suites: **430**
- Gate passing: **1** (0%)
- Gate failing: **429**
- Pending (no gate result yet): **0**

By family: FP-P2C **11** (1 gate-passing) · FP-P2D **419** (0 gate-passing)

**Compact status by packet** (✓ gate-pass / ✗ gate-fail / · pending):

**FP-P2C (Series-level):**

`FP-P2C-001 ✓`  `FP-P2C-002 ✗`  `FP-P2C-003 ✗`  `FP-P2C-004 ✗`  `FP-P2C-005 ✗`  `FP-P2C-006 ✗`  
`FP-P2C-007 ✗`  `FP-P2C-008 ✗`  `FP-P2C-009 ✗`  `FP-P2C-010 ✗`  `FP-P2C-011 ✗`  

**FP-P2D (DataFrame-level):**

`FP-P2D-014 ✗`  `FP-P2D-015 ✗`  `FP-P2D-016 ✗`  `FP-P2D-017 ✗`  `FP-P2D-018 ✗`  `FP-P2D-019 ✗`  
`FP-P2D-020 ✗`  `FP-P2D-021 ✗`  `FP-P2D-022 ✗`  `FP-P2D-023 ✗`  `FP-P2D-024 ✗`  `FP-P2D-025 ✗`  
`FP-P2D-026 ✗`  `FP-P2D-027 ✗`  `FP-P2D-028 ✗`  `FP-P2D-029 ✗`  `FP-P2D-030 ✗`  `FP-P2D-031 ✗`  
`FP-P2D-032 ✗`  `FP-P2D-033 ✗`  `FP-P2D-034 ✗`  `FP-P2D-035 ✗`  `FP-P2D-036 ✗`  `FP-P2D-037 ✗`  
`FP-P2D-038 ✗`  `FP-P2D-039 ✗`  `FP-P2D-040 ✗`  `FP-P2D-041 ✗`  `FP-P2D-042 ✗`  `FP-P2D-043 ✗`  
`FP-P2D-044 ✗`  `FP-P2D-045 ✗`  `FP-P2D-046 ✗`  `FP-P2D-047 ✗`  `FP-P2D-048 ✗`  `FP-P2D-049 ✗`  
`FP-P2D-050 ✗`  `FP-P2D-051 ✗`  `FP-P2D-052 ✗`  `FP-P2D-053 ✗`  `FP-P2D-054 ✗`  `FP-P2D-055 ✗`  
`FP-P2D-056 ✗`  `FP-P2D-057 ✗`  `FP-P2D-058 ✗`  `FP-P2D-060 ✗`  `FP-P2D-061 ✗`  `FP-P2D-062 ✗`  
`FP-P2D-063 ✗`  `FP-P2D-064 ✗`  `FP-P2D-065 ✗`  `FP-P2D-066 ✗`  `FP-P2D-067 ✗`  `FP-P2D-068 ✗`  
`FP-P2D-069 ✗`  `FP-P2D-070 ✗`  `FP-P2D-071 ✗`  `FP-P2D-072 ✗`  `FP-P2D-073 ✗`  `FP-P2D-074 ✗`  
`FP-P2D-075 ✗`  `FP-P2D-076 ✗`  `FP-P2D-077 ✗`  `FP-P2D-078 ✗`  `FP-P2D-079 ✗`  `FP-P2D-080 ✗`  
`FP-P2D-081 ✗`  `FP-P2D-082 ✗`  `FP-P2D-083 ✗`  `FP-P2D-084 ✗`  `FP-P2D-085 ✗`  `FP-P2D-086 ✗`  
`FP-P2D-087 ✗`  `FP-P2D-088 ✗`  `FP-P2D-089 ✗`  `FP-P2D-090 ✗`  `FP-P2D-091 ✗`  `FP-P2D-092 ✗`  
`FP-P2D-093 ✗`  `FP-P2D-094 ✗`  `FP-P2D-095 ✗`  `FP-P2D-096 ✗`  `FP-P2D-097 ✗`  `FP-P2D-098 ✗`  
`FP-P2D-099 ✗`  `FP-P2D-100 ✗`  `FP-P2D-101 ✗`  `FP-P2D-102 ✗`  `FP-P2D-103 ✗`  `FP-P2D-104 ✗`  
`FP-P2D-105 ✗`  `FP-P2D-106 ✗`  `FP-P2D-107 ✗`  `FP-P2D-108 ✗`  `FP-P2D-109 ✗`  `FP-P2D-110 ✗`  
`FP-P2D-111 ✗`  `FP-P2D-112 ✗`  `FP-P2D-113 ✗`  `FP-P2D-114 ✗`  `FP-P2D-115 ✗`  `FP-P2D-116 ✗`  
`FP-P2D-117 ✗`  `FP-P2D-118 ✗`  `FP-P2D-119 ✗`  `FP-P2D-120 ✗`  `FP-P2D-121 ✗`  `FP-P2D-122 ✗`  
`FP-P2D-123 ✗`  `FP-P2D-124 ✗`  `FP-P2D-125 ✗`  `FP-P2D-126 ✗`  `FP-P2D-127 ✗`  `FP-P2D-128 ✗`  
`FP-P2D-129 ✗`  `FP-P2D-130 ✗`  `FP-P2D-131 ✗`  `FP-P2D-132 ✗`  `FP-P2D-133 ✗`  `FP-P2D-134 ✗`  
`FP-P2D-135 ✗`  `FP-P2D-136 ✗`  `FP-P2D-137 ✗`  `FP-P2D-138 ✗`  `FP-P2D-139 ✗`  `FP-P2D-140 ✗`  
`FP-P2D-141 ✗`  `FP-P2D-142 ✗`  `FP-P2D-143 ✗`  `FP-P2D-144 ✗`  `FP-P2D-145 ✗`  `FP-P2D-146 ✗`  
`FP-P2D-147 ✗`  `FP-P2D-148 ✗`  `FP-P2D-149 ✗`  `FP-P2D-150 ✗`  `FP-P2D-151 ✗`  `FP-P2D-152 ✗`  
`FP-P2D-153 ✗`  `FP-P2D-154 ✗`  `FP-P2D-155 ✗`  `FP-P2D-156 ✗`  `FP-P2D-157 ✗`  `FP-P2D-158 ✗`  
`FP-P2D-159 ✗`  `FP-P2D-160 ✗`  `FP-P2D-161 ✗`  `FP-P2D-162 ✗`  `FP-P2D-163 ✗`  `FP-P2D-164 ✗`  
`FP-P2D-165 ✗`  `FP-P2D-166 ✗`  `FP-P2D-167 ✗`  `FP-P2D-168 ✗`  `FP-P2D-169 ✗`  `FP-P2D-170 ✗`  
`FP-P2D-171 ✗`  `FP-P2D-172 ✗`  `FP-P2D-173 ✗`  `FP-P2D-174 ✗`  `FP-P2D-175 ✗`  `FP-P2D-176 ✗`  
`FP-P2D-177 ✗`  `FP-P2D-178 ✗`  `FP-P2D-179 ✗`  `FP-P2D-180 ✗`  `FP-P2D-181 ✗`  `FP-P2D-182 ✗`  
`FP-P2D-183 ✗`  `FP-P2D-184 ✗`  `FP-P2D-185 ✗`  `FP-P2D-186 ✗`  `FP-P2D-187 ✗`  `FP-P2D-188 ✗`  
`FP-P2D-189 ✗`  `FP-P2D-190 ✗`  `FP-P2D-191 ✗`  `FP-P2D-192 ✗`  `FP-P2D-193 ✗`  `FP-P2D-194 ✗`  
`FP-P2D-195 ✗`  `FP-P2D-196 ✗`  `FP-P2D-197 ✗`  `FP-P2D-198 ✗`  `FP-P2D-199 ✗`  `FP-P2D-200 ✗`  
`FP-P2D-201 ✗`  `FP-P2D-202 ✗`  `FP-P2D-203 ✗`  `FP-P2D-204 ✗`  `FP-P2D-205 ✗`  `FP-P2D-206 ✗`  
`FP-P2D-207 ✗`  `FP-P2D-208 ✗`  `FP-P2D-209 ✗`  `FP-P2D-210 ✗`  `FP-P2D-211 ✗`  `FP-P2D-212 ✗`  
`FP-P2D-213 ✗`  `FP-P2D-214 ✗`  `FP-P2D-215 ✗`  `FP-P2D-216 ✗`  `FP-P2D-217 ✗`  `FP-P2D-218 ✗`  
`FP-P2D-219 ✗`  `FP-P2D-220 ✗`  `FP-P2D-221 ✗`  `FP-P2D-222 ✗`  `FP-P2D-223 ✗`  `FP-P2D-224 ✗`  
`FP-P2D-225 ✗`  `FP-P2D-226 ✗`  `FP-P2D-227 ✗`  `FP-P2D-228 ✗`  `FP-P2D-229 ✗`  `FP-P2D-230 ✗`  
`FP-P2D-231 ✗`  `FP-P2D-232 ✗`  `FP-P2D-233 ✗`  `FP-P2D-234 ✗`  `FP-P2D-235 ✗`  `FP-P2D-236 ✗`  
`FP-P2D-237 ✗`  `FP-P2D-238 ✗`  `FP-P2D-239 ✗`  `FP-P2D-240 ✗`  `FP-P2D-241 ✗`  `FP-P2D-242 ✗`  
`FP-P2D-243 ✗`  `FP-P2D-244 ✗`  `FP-P2D-245 ✗`  `FP-P2D-246 ✗`  `FP-P2D-247 ✗`  `FP-P2D-248 ✗`  
`FP-P2D-249 ✗`  `FP-P2D-250 ✗`  `FP-P2D-251 ✗`  `FP-P2D-252 ✗`  `FP-P2D-253 ✗`  `FP-P2D-254 ✗`  
`FP-P2D-255 ✗`  `FP-P2D-256 ✗`  `FP-P2D-257 ✗`  `FP-P2D-258 ✗`  `FP-P2D-259 ✗`  `FP-P2D-260 ✗`  
`FP-P2D-261 ✗`  `FP-P2D-262 ✗`  `FP-P2D-263 ✗`  `FP-P2D-264 ✗`  `FP-P2D-265 ✗`  `FP-P2D-266 ✗`  
`FP-P2D-267 ✗`  `FP-P2D-268 ✗`  `FP-P2D-269 ✗`  `FP-P2D-270 ✗`  `FP-P2D-271 ✗`  `FP-P2D-272 ✗`  
`FP-P2D-273 ✗`  `FP-P2D-274 ✗`  `FP-P2D-275 ✗`  `FP-P2D-276 ✗`  `FP-P2D-277 ✗`  `FP-P2D-278 ✗`  
`FP-P2D-279 ✗`  `FP-P2D-280 ✗`  `FP-P2D-281 ✗`  `FP-P2D-282 ✗`  `FP-P2D-283 ✗`  `FP-P2D-284 ✗`  
`FP-P2D-285 ✗`  `FP-P2D-286 ✗`  `FP-P2D-287 ✗`  `FP-P2D-288 ✗`  `FP-P2D-289 ✗`  `FP-P2D-290 ✗`  
`FP-P2D-291 ✗`  `FP-P2D-292 ✗`  `FP-P2D-293 ✗`  `FP-P2D-294 ✗`  `FP-P2D-295 ✗`  `FP-P2D-296 ✗`  
`FP-P2D-297 ✗`  `FP-P2D-298 ✗`  `FP-P2D-299 ✗`  `FP-P2D-300 ✗`  `FP-P2D-301 ✗`  `FP-P2D-302 ✗`  
`FP-P2D-303 ✗`  `FP-P2D-304 ✗`  `FP-P2D-305 ✗`  `FP-P2D-306 ✗`  `FP-P2D-307 ✗`  `FP-P2D-308 ✗`  
`FP-P2D-309 ✗`  `FP-P2D-310 ✗`  `FP-P2D-311 ✗`  `FP-P2D-312 ✗`  `FP-P2D-313 ✗`  `FP-P2D-314 ✗`  
`FP-P2D-315 ✗`  `FP-P2D-316 ✗`  `FP-P2D-317 ✗`  `FP-P2D-318 ✗`  `FP-P2D-319 ✗`  `FP-P2D-320 ✗`  
`FP-P2D-321 ✗`  `FP-P2D-322 ✗`  `FP-P2D-323 ✗`  `FP-P2D-324 ✗`  `FP-P2D-325 ✗`  `FP-P2D-326 ✗`  
`FP-P2D-327 ✗`  `FP-P2D-328 ✗`  `FP-P2D-329 ✗`  `FP-P2D-330 ✗`  `FP-P2D-331 ✗`  `FP-P2D-332 ✗`  
`FP-P2D-333 ✗`  `FP-P2D-334 ✗`  `FP-P2D-335 ✗`  `FP-P2D-336 ✗`  `FP-P2D-337 ✗`  `FP-P2D-338 ✗`  
`FP-P2D-339 ✗`  `FP-P2D-340 ✗`  `FP-P2D-341 ✗`  `FP-P2D-342 ✗`  `FP-P2D-343 ✗`  `FP-P2D-344 ✗`  
`FP-P2D-345 ✗`  `FP-P2D-346 ✗`  `FP-P2D-347 ✗`  `FP-P2D-348 ✗`  `FP-P2D-349 ✗`  `FP-P2D-350 ✗`  
`FP-P2D-351 ✗`  `FP-P2D-352 ✗`  `FP-P2D-353 ✗`  `FP-P2D-354 ✗`  `FP-P2D-355 ✗`  `FP-P2D-356 ✗`  
`FP-P2D-357 ✗`  `FP-P2D-358 ✗`  `FP-P2D-359 ✗`  `FP-P2D-360 ✗`  `FP-P2D-361 ✗`  `FP-P2D-362 ✗`  
`FP-P2D-363 ✗`  `FP-P2D-364 ✗`  `FP-P2D-365 ✗`  `FP-P2D-366 ✗`  `FP-P2D-367 ✗`  `FP-P2D-368 ✗`  
`FP-P2D-369 ✗`  `FP-P2D-370 ✗`  `FP-P2D-371 ✗`  `FP-P2D-372 ✗`  `FP-P2D-373 ✗`  `FP-P2D-374 ✗`  
`FP-P2D-375 ✗`  `FP-P2D-376 ✗`  `FP-P2D-377 ✗`  `FP-P2D-378 ✗`  `FP-P2D-379 ✗`  `FP-P2D-380 ✗`  
`FP-P2D-381 ✗`  `FP-P2D-382 ✗`  `FP-P2D-383 ✗`  `FP-P2D-384 ✗`  `FP-P2D-385 ✗`  `FP-P2D-386 ✗`  
`FP-P2D-387 ✗`  `FP-P2D-388 ✗`  `FP-P2D-389 ✗`  `FP-P2D-390 ✗`  `FP-P2D-391 ✗`  `FP-P2D-392 ✗`  
`FP-P2D-393 ✗`  `FP-P2D-394 ✗`  `FP-P2D-395 ✗`  `FP-P2D-396 ✗`  `FP-P2D-397 ✗`  `FP-P2D-398 ✗`  
`FP-P2D-399 ✗`  `FP-P2D-400 ✗`  `FP-P2D-401 ✗`  `FP-P2D-402 ✗`  `FP-P2D-403 ✗`  `FP-P2D-404 ✗`  
`FP-P2D-405 ✗`  `FP-P2D-406 ✗`  `FP-P2D-407 ✗`  `FP-P2D-408 ✗`  `FP-P2D-409 ✗`  `FP-P2D-410 ✗`  
`FP-P2D-411 ✗`  `FP-P2D-412 ✗`  `FP-P2D-413 ✗`  `FP-P2D-414 ✗`  `FP-P2D-415 ✗`  `FP-P2D-416 ✗`  
`FP-P2D-417 ✗`  `FP-P2D-418 ✗`  `FP-P2D-419 ✗`  `FP-P2D-420 ✗`  `FP-P2D-421 ✗`  `FP-P2D-422 ✗`  
`FP-P2D-423 ✗`  `FP-P2D-424 ✗`  `FP-P2D-425 ✗`  `FP-P2D-426 ✗`  `FP-P2D-427 ✗`  `FP-P2D-428 ✗`  
`FP-P2D-429 ✗`  `FP-P2D-430 ✗`  `FP-P2D-431 ✗`  `FP-P2D-432 ✗`  `FP-P2D-433 ✗`  

**Failing packets (showing first 25 of 429):**

| Packet | First reason |
|---|---|
| `FP-P2C-002` | failed=8 but gate requires 0 |
| `FP-P2C-003` | failed=3 but gate requires 0 |
| `FP-P2C-004` | failed=4 but gate requires 0 |
| `FP-P2C-005` | failed=3 but gate requires 0 |
| `FP-P2C-006` | failed=2 but gate requires 0 |
| `FP-P2C-007` | failed=3 but gate requires 0 |
| `FP-P2C-008` | failed=2 but gate requires 0 |
| `FP-P2C-009` | failed=2 but gate requires 0 |
| `FP-P2C-010` | failed=25 but gate requires 0 |
| `FP-P2C-011` | failed=21 but gate requires 0 |
| `FP-P2D-014` | failed=12 but gate requires 0 |
| `FP-P2D-015` | failed=14 but gate requires 0 |
| `FP-P2D-016` | failed=14 but gate requires 0 |
| `FP-P2D-017` | failed=14 but gate requires 0 |
| `FP-P2D-018` | failed=18 but gate requires 0 |
| `FP-P2D-019` | failed=14 but gate requires 0 |
| `FP-P2D-020` | failed=14 but gate requires 0 |
| `FP-P2D-021` | failed=17 but gate requires 0 |
| `FP-P2D-022` | failed=17 but gate requires 0 |
| `FP-P2D-023` | failed=17 but gate requires 0 |
| `FP-P2D-024` | failed=13 but gate requires 0 |
| `FP-P2D-025` | failed=13 but gate requires 0 |
| `FP-P2D-026` | failed=10 but gate requires 0 |
| `FP-P2D-027` | failed=10 but gate requires 0 |
| `FP-P2D-028` | failed=10 but gate requires 0 |
| ... | _and 404 more — see `artifacts/phase2c/drift_history.jsonl`_ |

<!-- END AUTO-PACKET-TABLE -->
