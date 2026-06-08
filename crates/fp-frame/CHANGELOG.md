# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/fp-frame-v0.1.0) - 2026-06-08

### <!-- 0 -->Added

- *(fp-index)* typed missing index label IndexLabel::Null(NullKind) (br-frankenpandas-joeff)
- *(frame)* GroupBy head/tail support pandas negative n
- *(frame)* str.slice_replace supports Python negative start/stop
- *(frame)* str.slice supports full Python slice semantics (negatives + step)
- *(frame)* str.get supports Python-style negative indexing
- *(frame)* DataFrame.clip per-row Series bounds (axis=0) (br-frankenpandas-l4wfl)
- *(frame)* DataFrame.clip per-column Series bounds (axis=1) (br-frankenpandas-l4wfl)
- *(frame)* DataFrame.compare align_axis=0 row-MultiIndex stacking (br-frankenpandas-lqu84)
- *(frame)* DataFrame.compare supports keep_shape/keep_equal (br-frankenpandas-lqu84)
- *(frame)* pivot_table supports nunique/sem/skew aggfuncs (br-frankenpandas-464hw)
- *(fp-frame,fp-io)* add Series-level asfreq/to_period/to_xarray/to_hdf
- *(fp-frame)* add parse_resample_freq and day-origin bucketing helpers
- *(fp-frame)* close Series metadata surface [frankenpandas-r6uci]
- *(fp-frame)* complete window API aliases [frankenpandas-073ty]
- *(fp-frame)* Series::cov_with_options(other, min_periods, ddof)
- *(fp-frame)* Series::quantile_with_interpolation (linear/lower/higher/nearest/midpoint)
- *(fp-frame)* Series::factorize_with_options(sort, use_na_sentinel)
- *(fp-frame)* expand Resampler parity
- *(fp-frame)* expand DataFrameGroupBy parity
- *(fp-frame)* add Series pandas aliases
- *(fp-frame)* expand SeriesGroupBy parity surface (frankenpandas-nt65g)
- *(fp-frame)* add pandas-parity df.min/max/std/var/etc. methods (fd90.233 / br-he7yj)
- *(fp-frame)* add DataFrameResample::first/last (fd90.200 / br-kzmxd)
- *(fp-frame)* add where_mask_df_other to mirror mask_df_other (br-df9v7 / fd90.138)
- *(frankenpandas)* re-export rusqlite under sql-sqlite feature (br-r34z9 / fd90.130)
- *(frankenpandas)* expand prelude with path-based IO + add Path import to Recipes (br-m3zib / fd90.125)
- *(fp-frame)* expand groupby apply result shapes (frankenpandas-0kx7)
- *(fp-frame)* add Series sparse accessor (frankenpandas-0xcm.2)
- *(io)* gate sqlite sql backend (br-frankenpandas-fd90)

### <!-- 1 -->Fixed

- *(validation)* keep scoped all-target gates green
- *(fp-frame)* plain pivot and crosstab sort key axes like pandas (br-frankenpandas-r0t9l)
- *(fp-frame)* pivot_table sorts both axes ascending with nulls last (br-frankenpandas-s3x7k)
- *(fp-frame,fp-io)* typed null labels for set_index, index_col, pivot and categorical buckets (br-frankenpandas-8m6ay complete)
- *(fp-frame)* value_counts(dropna=False) keeps None/nan/NaT as distinct buckets (br-frankenpandas-joeff phase 2)
- *(frame)* DataFrame from_series keeps last series for a duplicate name
- *(frame)* DataFrame from dict-of-series sorts the union index like pandas
- *(frame)* pct_change axis=1 and groupby forward-fill like pandas
- *(frame)* dt.round/floor/ceil work in nanoseconds (sub-second freqs + ties)
- *(frame)* str split_get/rsplit_get/split_regex_get support negative index
- *(frame)* DataFrame.groupby().sum() concatenates string columns like pandas
- *(columnar,frame)* Bool.diff() is XOR (pandas 2.2.3), not numeric subtraction
- *(frame)* Series &/| and DataFrame.combine return the sorted union like pandas
- *(frame)* combine_first returns the sorted union index/columns like pandas
- *(frame)* DataFrame.value_counts breaks count ties by key asc, not first-occurrence
- *(frame)* str.rsplit_get returns parts in forward (pandas) order, not reversed
- *(frame)* DataFrame/scalar/timedelta mod sign + div-by-zero across non-kernel paths
- *(frame)* fill_value div/floordiv by zero -> +/-inf; mod uses divisor sign
- *(frame)* value_counts(dropna=False) null bucket at first-seen position
- *(frame)* isin matches missing values by NullKind, not any-missing
- *(frame)* str.pad(both)/center match CPython center odd-padding tie
- *(frame)* dt.round breaks exact ties half-to-even to match pandas
- *(frame)* clip swaps reversed scalar bounds to match pandas (GH 2747)
- *(frame)* Series.unique()/nunique(dropna=False) key nulls by NullKind (9x4qi)
- *(frame)* quantile interpolation='nearest' rounds index half-to-even
- *(frame)* reset_index restores the index under its name, not always 'index' (br-frankenpandas-evj32)
- *(frame)* preserve datetime dtype on reset_index/index extraction (br-frankenpandas-cxynt)
- *(frame)* set_index accepts Datetime64/Timedelta64 columns (br-frankenpandas-cxynt)
- *(frame)* support Weekly/Business period freqs in to_timestamp (br-frankenpandas-13cv5)
- *(repr)* bool True/False in csv-read index, multi-index, get_dummies cols
- *(repr)* render bool as True/False (pandas parity), not lowercase
- *(frame)* DataFrame groupby kurtosis uses correct Fisher G2 formula
- *(get_dummies)* emit bool indicator columns for pandas 2.x parity
- *(frame)* Series.equals ignores name (pandas parity) ()
- *(frame)* interpolate forward-fills trailing NaNs (pandas parity) (br-frankenpandas-v8kng)
- *(frame)* cut/qcut bin labels match pandas (widen first edge + 3 sig-figs) (br-4rfy1)
- *(frame)* to_timedelta bare numeric defaults to ns + truncates (br-0caxj)
- *(frame)* groupby.skew bias correction matches pandas (br-frankenpandas-c7feq)
- *(frame)* rank pct/na_option ranks null group by method (pandas parity)
- dt.round, rolling count, Excel round-trip, dt.total_seconds parity
- resolve P1 filter regression + rolling/ewm min_periods bugs
- close multiple conformance/feature beads
- *(fp-frame)* ensure combine_first dtype promotion is associative
- *(fp-frame/fp-types)* dt.total_seconds() for timedelta dtype (pandas parity)
- *(fp-frame)* honor min_periods for partial rolling windows (pandas parity)
- *(fp-frame)* promote to Float64 on series alignment gaps (pandas parity)
- *(fp-frame)* GroupBy::idxmin/idxmax handle Utf8 series via lex compare
- *(fp-frame)* clear clippy after info golden co-land
- *(fp-frame)* cumsum/cummin/cummax handle Utf8+Null mix instead of erroring
- *(fp-frame)* pandas-canonical .0 suffix for whole-number Float64 in to_csv
- *(fp-frame)* td_* family errors on non-Timedelta operands instead of silent NAT/false
- *(fp-frame)* guard string split expansion limit overflow
- *(fp-frame)* DatetimeAccessor errors on non-datetimelike Series instead of silent NaN
- *(fp-frame)* str.find/rfind/index_of/rindex_of return CHAR not BYTE position
- *(fp-frame)* clean up CSV NA test options (frankenpandas-660di)
- *(fp-frame)* validate na_position in sort_values_na (frankenpandas-2ca7d)
- *(fp-frame)* validate groupby sample frac (frankenpandas-h47vh)
- *(fp-frame)* return NaN for undersized Series sem (frankenpandas-axms5)
- *(fp-frame)* rank Utf8 lexicographically instead of silently NaN [br-frankenpandas-ff284]
- *(frame)* remove duplicate DataFrame bool accessor
- *(fp-frame)* include memory usage in DataFrame info (frankenpandas-cod1m04)
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

### <!-- 2 -->Performance

- *(bench)* dropna harness + record rejected typed-missing-count lever
- *(fp-frame)* dense-gid typed groupby shift (positive periods, Series + DataFrame) — ~7.3x
- *(fp-frame)* dense-gid typed groupby diff (Series + DataFrame) — ~6-7x
- *(fp-frame)* dense-gid typed DataFrameGroupBy cumsum/cumprod/cummin/cummax — ~7x
- *(fp-frame)* dense-gid typed groupby cumsum/cumprod/cummin/cummax — ~7x
- *(fp-frame)* dense groupby.transform for single contiguous-Utf8 keys — 7.6x
- *(fp-frame)* dense groupby.transform var/std/sum — ~7x std (br-frankenpandas-sj0wq)
- *(fp-frame)* extend dense groupby.transform to median/first/last/prod — 3.9x median (br-frankenpandas-8kags follow-up)
- *(fp-frame)* dense-gid typed groupby.transform (mean/count/min/max) — 8.4x (br-frankenpandas-8kags)
- *(fp-frame)* typed all-valid-Float64 rank fast path — 1.34x (br-frankenpandas-wtdor)
- *(fp-frame)* radix Series.sort_index for all-Int64 index — ~5.5x (br-frankenpandas-d9joc)
- *(fp-frame)* radix sort_index for all-Int64 index — 3.9x (br-frankenpandas-y5s15)
- *(fp-frame)* stable radix lexsort for sort_values_multi — 2.9x (br-frankenpandas-lnsu6)
- *(fp-frame)* typed-comparison multi-column sort_values_multi — 1.57x (br-frankenpandas-1tuf5)
- *(fp-frame)* narrow Kendall rank/order arrays + Fenwick to u32 — 1.21x (br-frankenpandas-322yd)
- *(fp-frame)* uncap correlation-matrix workers 8->cores (kendall 1.34x / spearman 1.43x, bandwidth-bound) (br-frankenpandas-wck31)
- *(fp-frame)* fast-sort contiguous Utf8 Series (br-frankenpandas-3hna0)
- *(fp-frame)* complete str-groupby build_groups-free dense bypass — var/std/first/last/prod/median ~2.1x (br-frankenpandas-tpulz)
- *(fp-frame)* str-groupby build_groups-free dense mean/count/min/max — 2.3-2.5x (br-frankenpandas-ygt39)
- *(fp-frame)* contiguous-Utf8 byte-span groupby — 4.19x str_groupby_sum (br-frankenpandas-s7b7q)
- *(fp-frame)* hoist key-column slices + single-probe groupby build
- *(fp-frame)* fast-sort contiguous Utf8 keys (br-frankenpandas-vecff)
- *(fp-frame)* ASCII fast-path in-place case mapping for str.lower/upper (br-frankenpandas-2krr0 rung 6)
- *(fp-frame)* SIMD whole-buffer scan for str.contains on contiguous Utf8 (br-frankenpandas-2krr0 rung 4)
- *(fp-columnar,fp-frame)* chained string ops read contiguous Utf8 end to end — 1.42x on pipelines (br-frankenpandas-2krr0 rung 3)
- *(fp-columnar,fp-frame)* contiguous Utf8 output for string ops — closes the str.lower pandas gap (br-frankenpandas-2krr0 rung 2)
- *(fp-frame)* typed Int64 output + Arc index reuse for int-returning string ops — 3.36x (br-frankenpandas-2krr0 rung 1)
- *(fp-frame)* typed bool output + Arc index reuse for string predicates — 3.06x (str-scan campaign opener)
- *(fp-index)* typed Int64 IndexLabels storage + i64 label gather (br-frankenpandas-dxqpm)
- *(fp-frame)* lazy unit-range default RangeIndex at 4 construction sites (br-frankenpandas-arr72)
- record rejected fused mask filter; filter_bool gap is closed (br-frankenpandas-fi6zx)
- range-back shifted Int64 union index (br-frankenpandas-uza04.23)
- *(fp-frame)* speed Kendall inversion count 1.52x
- *(fp-frame)* parallelize complete Kendall cells
- *(frame)* parallelize complete Spearman rank precompute
- *(frame)* typed Column::take_positions gather for DataFrame iloc/take/sample — 2.45x
- *(frame)* typed take_positions gather for Series.take + groupby selectors — 2.26x
- *(frame)* typed Column::take_positions gather for Series iloc/reorder — 2.78x
- *(frame)* single-pass anchored regex for str.startswith_any/endswith_any — 4.8x
- *(frame)* single-pass Aho-Corasick for Series.str.contains_any — 17.3x
- *(frame)* O(n) char-map for Series.str.translate — 33.7x
- *(frame)* O(n) row-match map for DataFrame.corrwith(axis=1) — 102x
- *(frame)* O(m+n) label-position map for DataFrame.loc list selector — 149x
- *(frame)* binary-search Series.searchsorted_values for sorted input — 348x
- *(frame)* Knight O(n log n) Kendall tau-b for tied data — 107x
- *(frame)* O(m+n) label-position map for Series/DataFrame list-loc — 148x
- fuse monotone Series alignment (br-frankenpandas-uza04.22)
- bypass exact-index Series alignment maps (br-frankenpandas-uza04.18)
- *(frame)* dense-gid SeriesGroupBy build_groups — 4.2x
- *(frame)* counting-histogram O(n) Series.rank — ~3x
- *(frame)* dense seen-bitset drop_duplicates — 10-16x
- *(frame)* dense seen-bitset duplicated/duplicated_keep — 2.4-2.9x
- *(frame)* hash-free direct-address mode — 6-12x
- *(frame)* hash-free direct-address factorize — 4-5x
- *(frame)* hash-free direct-address value_counts — 13-16x
- *(frame)* binary-search weighted sample CDF — 124x
- *(frame)* scatter + typed-i64 str.get_dummies — 2.11x
- *(frame)* scatter pivot_table output build — 2.59x
- *(frame)* scatter + typed-bool get_dummies — 2.08x
- *(frame)* crosstab O(n^2)->O(n) dedup + scatter output — 4.07x
- *(frame)* parallelize complete Spearman cells
- *(frame)* O(n log U) expanding rank — 2279x
- *(frame)* O(n) expanding sum/mean — 240x
- *(frame)* centered rolling min/max/median/quantile incremental too
- *(frame)* partial-select nlargest/nsmallest — up to 18.9x
- *(frame)* incremental expanding min/max/median/quantile — O(n^2)->O(n log)
- *(frame)* O(n log U) Fenwick rolling median/quantile — ~15x
- *(frame)* typed corr/cov extraction on blocked Gram
- *(frame)* blocked-Gram fast path for df.corr/cov — 2.24x
- *(frame)* rank-domain Kendall inversion
- *(frame)* remove rejected range alignment fast path
- *(frame)* typed O(n) quickselect for Series.quantile
- *(frame)* typed O(n) quickselect for Series.median — 10.4x
- *(frame)* typed prefix fast paths for cumprod/cummin/cummax
- *(frame)* typed prefix-sum fast path for Series.cumsum — 4.2x
- *(frame)* dense groupby prod
- *(frame)* dense groupby first/last
- *(frame)* dense groupby agg for multi-column composite-int keys
- *(frame)* multi-column composite-int dense build_groups — 19.3x
- *(frame)* dense groupby Int64 sum/mean (i128) — completes dense agg
- *(frame)* dense groupby var/std (two-pass, ddof=1)
- *(frame)* dense groupby min/max (Float64+Int64) + Int64 count
- *(frame)* dense single-pass DataFrameGroupBy sum/mean/count — 3.5x agg
- *(frame)* dense build_groups for DataFrameGroupBy single int key — 22.7x grouping
- *(frame)* dense-bucket SeriesGroupBy agg_numeric — 12.8x
- *(frame)* typed DataFrame scalar arithmetic — 4-6x
- *(frame)* validity-mask read for DataFrame.isna/notna — 8-21x
- *(frame)* validity-mask read for Series.isna/notna — 7.1x
- *(frame)* hash-free bitset + typed output for DataFrame.isin — 20x
- *(frame)* typed compare + bool output for Series.between — 3.84x
- *(frame)* hash-free bitset isin + typed bool output — 4.7-5.1x
- *(frame)* hash-free dense dedup for Series.unique/nunique — 16-19x
- *(frame)* hash-free dense-histogram value_counts — 2.9-17.1x
- *(frame)* route user-facing Series.sort_values through typed radix — 2.40x
- *(frame)* reuse Kendall sorted order witnesses
- *(frame)* cache centered ranks for Spearman corr
- *(frame)* use unstable Spearman rank sort
- *(frame)* prepack dense Kendall columns
- *(frame)* accelerate Kendall tau matrix
- *(csv)* route default options through fast parser
- *(series)* cache sorted-union witness fingerprints (br-frankenpandas-eb1pb)
- *(frame)* typed Float64 reduction for Series.sum — 5.02x
- *(frame)* cache unique alignment indexes (br-frankenpandas-typed-columnar-storage-epic-fi6zx.15)
- *(frame)* stream integer index witness hashes
- *(frame)* typed sortedness for dense sort keys
- *(frame)* skip duplicate hashing for unique selected columns
- *(frame)* skip gather for all-unique drop_duplicates
- *(frame)* precompute Spearman ranks once per column — 69x on corr matrix (dbxrn)
- *(frame)* halve DataFrame.corr(spearman/kendall) via exact symmetry (dbxrn)
- *(frame)* halve DataFrame.corr()/cov() pairwise work, bit-identical (m0c5i)
- *(frame)* fast-path already sorted sort_values
- *(frame)* fuse aligned float series arithmetic
- *(frame)* fast-path sorted AACE union alignment (br-frankenpandas-lzklz)
- *(frame)* skip AACE witness fingerprint for discarded-ledger arithmetic (br-b75cc)
- *(frame)* hash duplicated rows without row key vectors (br-frankenpandas-fgpx3)
- *(frame)* add dense numeric sort fast path (br-frankenpandas-uxkvh)
- *(frame)* fast-path row gather materialization (br-frankenpandas-2a6ln)
- *(fp-frame)* optimize sort_values, filter_rows, reorder_rows
- *(fp-frame)* drop_duplicates O(n²) → O(n) hash-based algorithm
- perf-scorecard scaffold honest vs-pandas measurement infrastructure
- *(fp-frame)* select_columns O(|names|*|cols|) -> O(|names|+|cols|) via name->position HashMap
- *(fp-frame)* mode_values O(n²) -> O(n) via canonical ModeKey for cross-dtype bucketing
- *(fp-frame)* hoist between() inclusive dispatch out of per-element loop
- *(fp-frame)* normalize_column_order O(n²) → O(n+k) via last-idx HashMap + HashSet

### <!-- 3 -->Changed

- *(fp-frame)* remove dead DuplicateIndexUnsupported variant (br-yvkrb / fd90.145)
- *(fp-frame)* convert where_mask_df_other to delegator over where_cond_df (br-a2sck / fd90.139)

### <!-- 4 -->Documentation

- *(frame)* correct stale Bool-column ops comment to pandas 2.2.3 semantics
- refresh README with current conformance status
- *(readme)* fix two wrong paths in "How to Add a New Pandas Method"
- *(readme)* fix conformance-packet JSON layout — operation is a string
- *(readme)* GroupBy walkthrough — correct dense-path scope + fn names
- *(readme)* add missing 'sort' field to MergeExecutionOptions claim
- *(readme)* fix IO format table — wrong field names in 6 options structs
- *(readme)* fix Markdown tablefmt values (lowercase string, not capitalized)
- *(readme)* fix NanOps count (23 not 24) + EvidenceLedger method names
- *(readme)* fix CompatibilityIssue struct shape in glossary
- correct the "src/ LOC excluding tests_*.rs" miscount
- post-rewrite audit fixes — 4 structural bugs + drift refresh
- *(readme)* comprehensive rewrite + de-slopify pass for 2026-05-16 state
- *(fp-frame)* close plotting decision [frankenpandas-hci9o]
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

### <!-- 5 -->Testing

- *(fp-frame)* make Debug-format goldens robust to internal-repr churn (br-frankenpandas-esj4i)
- *(fp-frame)* refresh 3 Debug goldens for f9b4d259 all-valid ValidityMask repr — main green
- *(fp-frame)* refresh 3 Debug goldens for arr72 unit-range cache repr (br-frankenpandas-5gns6)
- *(frame)* strip non-deterministic Index label_identity from golden compares
- *(frame)* regenerate Index-debug goldens for semantic_fingerprint_cache
- *(frame)* refresh stale fillna_method golden to pandas-correct repr
- *(fp-frame)* expand Series golden statistical test coverage
- *(fp-frame)* cover Series alias surface
- *(fp-frame)* golden artifact for DataFrame::to_json("records") mixed dtypes
- *(fp-frame, fp-conformance)* externalize info() goldens + add unicode removeprefix oracle; compact stale beads
- *(fp-frame)* golden artifact for DataFrame::to_csv_options quoting + na_rep
- *(fp-frame)* golden artifact for DataFrame::to_markdown mixed dtypes
- *(fp-frame)* metamorphic property tests for Series sort/arith (skill: metamorphic)
- *(fp-frame)* freeze info() output as golden artifacts (skill: golden-artifacts)
- *(frame)* freeze display goldens (frankenpandas-cod1m03)
- *(frankenpandas)* add Missing Data Handling test + fix README dropna_with_threshold arity (br-jl1go / fd90.172)
- *(conformance)* add io formats to_html matrix
- *(conformance)* add MultiIndex parity matrix (br-frankenpandas-m8cp)

### <!-- 9 -->Reverted

- Revert "fp-frame DISC-014 fix duplicate-label arithmetic promotes Int64 to Float64"

### DataFrame

- :quantile_axis1 preserves Timedelta64 dtype
- :idxmin/idxmax skip non-numeric columns (FP-P2D-148)
- :mode + mode_axis1 numeric_only include Timedelta64 (br-frankenpandas-uy9z2)
- :corrwith + corrwith_axis include Timedelta64 (br-frankenpandas-ncuvp)
- :corr/cov include Timedelta64 columns (br-frankenpandas-qk3s0)
- :idxmin_axis1/idxmax_axis1 handle Timedelta64 (br-frankenpandas-dvzw1)
- :{sum,mean,min,max}_axis1 preserve Timedelta64 (br-frankenpandas-c0g3x)
- :cum{sum,prod,min,max}_axis1 preserve Timedelta64 (br-frankenpandas-bktp1)
- :reduce_numeric_skipna includes Timedelta64 (br-frankenpandas-4zg55)
- :std_agg_ddof + var_agg_ddof include Timedelta64 (br-frankenpandas-qin9h)
- :reduce_numeric includes Timedelta64 columns (br-frankenpandas-vpeoh)
- :pivot sets index name from index_col (frankenpandas-xb0ra)
- :unstack preserves source index name (frankenpandas-yh08x)
- :stack preserves source index name (frankenpandas-ecl0a)
- :rename_index (mapping) preserves axis name (frankenpandas-saitv)
- :sample_weights rejects negative/NaN weights (frankenpandas-s7o17)
- :groupby rejects empty by-list (frankenpandas-tm5xu)
- :reindex_axis(axis=1) preserves row_multiindex (frankenpandas-hn794)
- :asof uses searched label as Series name (frankenpandas-5hoh1)
- :truncate preserves index name (frankenpandas-oygjj)
- :drop_rows_int rejects missing labels (frankenpandas-372dn)
- :drop(axis=0) rejects missing row labels (frankenpandas-63dgc)
- :sample preserves index name (frankenpandas-omjaw)
- :rename_index_with preserves index name (frankenpandas-789xi)
- :insert rejects out-of-bounds loc (frankenpandas-wmucs)

### DataFrameGroupBy

- :idxmin/idxmax handle Timedelta64 columns (br-frankenpandas-41rnd)
- :sem preserves Timedelta64 (br-frankenpandas-5fbpy)
- :quantile preserves Timedelta64 (br-frankenpandas-v61uo)
- :cum{sum,prod,min,max} preserve Timedelta64 (br-frankenpandas-ccf67)

### Resample

- :var preserves Timedelta64 via fp_types::nanvar (br-frankenpandas-x3lk4)

### Rolling

- :max/prod return NaN on empty window (frankenpandas-5m7r1)
- :first/last: guard against all-missing window panic (frankenpandas-dvxj7)

### Series

- :clip_with_series preserves Int64 dtype when bounds are integer
- :cov_components handles Timedelta64 pairs (br-frankenpandas-of7v2)
- :corr_spearman + corr_kendall handle Timedelta64 (br-frankenpandas-xekug)
- :idxmin/idxmax handle Timedelta64 (br-frankenpandas-9kt88)
- :interpolate_method preserves Timedelta64 for nearest/zero (br-frankenpandas-1xtur)
- :interpolate preserves Timedelta64 dtype (br-frankenpandas-wax2l)
- :rank handles Timedelta64 as ordered numeric (br-frankenpandas-5pxmt)
- :cumprod returns all-NaT for Timedelta64 (br-frankenpandas-v36qy)
- :cummin + cummax preserve Timedelta64 dtype (br-frankenpandas-v7spg)
- :cumsum preserves Timedelta64 dtype (br-frankenpandas-gqrmf)
- :prod returns NaT for Timedelta64 (br-frankenpandas-mpw1f)
- :*_skipna(false) returns NaT for Timedelta64 (br-frankenpandas-fsrfm)
- :var_ddof + std_ddof preserve Timedelta64 (br-frankenpandas-e686u)
- :std takes sqrt of Timedelta64 variance (br-frankenpandas-j0ilf)
- :var preserves Timedelta64 dtype (br-frankenpandas-yy0ks)
- :mean preserves Timedelta64 dtype (br-frankenpandas-edmsd)
- :max preserves Timedelta64 dtype (br-frankenpandas-7iz85)
- :min preserves Timedelta64 dtype (br-frankenpandas-erobu)
- :sum preserves Timedelta64 dtype (br-frankenpandas-28lgk)
- :median preserves Timedelta64 dtype (br-frankenpandas-rbt10)
- :quantile preserves Timedelta64 dtype (br-frankenpandas-ppc2r)
- :reindex and DataFrame::reindex preserve index name (frankenpandas-1lo93)
- :drop rejects missing labels to match pandas KeyError (frankenpandas-dopex)
- :truncate preserves index name (frankenpandas-5cc2t)
- :nlargest/nsmallest preserve index name (frankenpandas-0trpd)
- :add_prefix/add_suffix preserve index name (frankenpandas-a1sfw)
- :value_counts result index.name = source series name (frankenpandas-nvglo)

### Series/DataFrame

- :first_offset use ok_or_else on labels[0] (frankenpandas-2xvmk)

### SeriesGroupBy

- :idxmin/idxmax handle Timedelta64 (br-frankenpandas-q6nxh)
- :cum{sum,prod,min,max} preserve Timedelta64 (br-frankenpandas-v6j38)
- :prod returns NaT for Timedelta64 (br-frankenpandas-s13rm)
- :std/var/median preserve Timedelta64 (br-frankenpandas-pwt7r)
- :min/max Timedelta64 fast path (frankenpandas-tgm2i)
- :sum/mean Timedelta64 fast path (frankenpandas-c1bxu)

### Ewm.sum

- guard against to_f64 errors to avoid Utf8 panic (frankenpandas-ntoqd)

### Fp-frame

- bench+golden example harnesses for kendall/loc/map_dict/sort_index
- add median aggregation to DataFrameGroupBy
- wire nullable Int64 through GroupBy aggregations
- fix assert_text_golden to normalize trailing newlines on both sides
- add aggregation, correlation, and metadata conformance tests
- add sorting, selection, and boolean conformance tests
- fix clippy too_many_arguments warning + fmt
- add Series transformation conformance tests
- add expanding, groupby, transpose, and utility conformance tests
- add string accessor, IO null/nan, and rolling edge case conformance tests
- add IO/rolling/expanding/reshape conformance tests
- add Series.str.isascii() for pandas parity
- add test for escapechar self-escaping
- add escapechar support for CsvQuoting::None
- add header parameter to to_csv methods
- add golden tests for str.ljust/rjust/wrap/normalize
- add golden tests for case_when, absolute, casefold, center
- add CsvQuoting enum and to_csv_with_quoting methods
- add golden tests for Series logical and bitwise operations
- DataFrame.mode() uses Float64 when NaN padding is needed
- add golden text-rendering tests for DataFrame head/tail/nlargest/nsmallest/drop_duplicates
- Add DatetimeAccessor is_weekday/is_weekend methods
- Add Series.mad() method for mean absolute deviation
- Add StringAccessor rsplit_expand/rsplit_expand_n methods
- Add doublequote/escapechar CSV options support
- Add Timedelta accessor methods to DatetimeAccessor
- Add StructAccessor.dtypes() method
- Add StructAccessor.explode() method
- Add ListAccessor.argmin() and argmax() methods
- Add ListAccessor.nunique() method
- Add ListAccessor.all() and any() methods
- Add ListAccessor.sort() method
- Add ListAccessor.reverse() method
- Add ListAccessor.first() and last() methods
- Add ListAccessor.count() method
- Add ListAccessor.median() method
- Add ListAccessor.var() method
- Add ListAccessor.std() method
- Add ListAccessor.prod() method
- Add CategoricalAccessor.min() and max() methods
- Add CategoricalAccessor.map() method
- Add ListAccessor.contains() method
- Add ListAccessor.join() method
- Add ListAccessor.min() and max() methods
- Add ListAccessor.mean() method
- Add ListAccessor.sum() method
- Add SparseAccessor.sp_index() method
- Add SparseAccessor.sp_values() method
- Add CategoricalAccessor.reorder_categories() method
- Add CategoricalAccessor.remove_categories() method
- Add DatetimeAccessor.components() method
- Add DatetimeAccessor.timetz() method
- Add DatetimeAccessor.tz() method
- Fix clippy useless_conversion warnings in isocalendar
- Add DatetimeAccessor.isocalendar() method
- Add pandas-compatible DatetimeAccessor aliases
- Add DatetimeAccessor.time() method
- Add DatetimeAccessor.normalize() method
- Stop Float64 promotion in mode aggregation (DISC-011)
- reject timedelta corrwith columns
- align rolling pairwise stats by index
- align Series pairwise correlations by index
- preserve source axis name in DatetimeAccessor::extract_component (br-frankenpandas-8pyft)
- SeriesGroupBy string min max parity (br-nt65g.12)
- SeriesGroupBy first last null parity (br-nt65g.11)
- SeriesGroupBy monotonic null parity (br-nt65g.10)
- SeriesGroupBy ohlc parity (br-nt65g.9)
- SeriesGroupBy unique parity (br-nt65g.8)
- SeriesGroupBy value counts parity (br-nt65g.7)
- SeriesGroupBy ranked selection parity (br-nt65g.6)
- SeriesGroupBy missing-value parity (br-nt65g.5)
- SeriesGroupBy quantile sem skew parity (br-nt65g.4)
- SeriesGroupBy sample parity (br-nt65g.3)
- SeriesGroupBy take parity (br-nt65g.2)
- SeriesGroupBy.transform / .filter / .pipe parity slice (br-nt65g.1)

### Fp-frame/fp-io

- Fix clippy warnings (unused import, needless_borrow)

### Fp-types

- add nullable Int64/Bool extension dtypes (DISC-011/014)
- Add Timedelta component accessor methods
- Add Timestamp.toordinal() and fromordinal() methods

### Style

- cargo fmt across the workspace
- *(frame)* drop redundant matches!(b, true) bool patterns (clippy)
- *(fp-frame)* cargo fmt reflow on 5110c
