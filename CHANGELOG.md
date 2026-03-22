# Changelog

All notable changes to FrankenPandas are documented in this file.

FrankenPandas is a clean-room Rust reimplementation of the full pandas API surface.
No tagged releases exist yet; this changelog is organized by development phase and date.

Repository: <https://github.com/Dicklesworthstone/frankenpandas>

---

## [Unreleased] (HEAD)

**Latest commit:** [`8dbb4c1`](https://github.com/Dicklesworthstone/frankenpandas/commit/8dbb4c100144b543d43888a31a267f9952eab98c) -- 2026-03-17

### 2026-03-17 -- Named Aggregation, Documentation Rewrite

#### Added
- **DataFrameGroupBy.agg_named()** for pandas-style named aggregation with `(output_name, column, func)` tuples ([`ef15a82`](https://github.com/Dicklesworthstone/frankenpandas/commit/ef15a825df581b9150fe50bd320cfdae5465ff30))

#### Fixed
- agg_named rejects duplicate output column names ([`fbf21b8`](https://github.com/Dicklesworthstone/frankenpandas/commit/fbf21b8fcb4ec39dc5bdd73ac1dd357f315dd946))
- Fix misaligned right-side borders in all 3 ASCII art architecture diagrams ([`8dbb4c1`](https://github.com/Dicklesworthstone/frankenpandas/commit/8dbb4c100144b543d43888a31a267f9952eab98c))

#### Docs
- Complete README.md rewrite to 1574 lines covering architecture, recipes, optimization catalog, error architecture, missing data semantics, coercion rules, element-wise ops, selection, and introspection ([`81c58c4`](https://github.com/Dicklesworthstone/frankenpandas/commit/81c58c4e2c3b7d9d303b74d9733932cefff62425)...[`95bc05e`](https://github.com/Dicklesworthstone/frankenpandas/commit/95bc05e04b60cbe13ba07938d6ef4bec32fb154c))

---

### 2026-03-16 -- Facade Crate, JSONL I/O, Datetime/Timedelta Parsing, MultiIndex Levels

#### Added -- I/O
- **JSONL (JSON Lines) read/write** I/O format (7th format) ([`30156ec`](https://github.com/Dicklesworthstone/frankenpandas/commit/30156ec2f0f24cb52f44778ce948fb3c9f6918d4))
- **CsvReadOptions** expanded with `usecols`, `nrows`, `skiprows`, `dtype` (pandas `read_csv` parity) ([`8e21aa5`](https://github.com/Dicklesworthstone/frankenpandas/commit/8e21aa54b44eb912a33298e4960d42c8bdbcce2f), [`f648113`](https://github.com/Dicklesworthstone/frankenpandas/commit/f6481139e08d2b0fb537ded6afab74ba0d83eb93))

#### Added -- Frame
- **pd.to_datetime()** equivalent for flexible datetime string parsing ([`e5557ec`](https://github.com/Dicklesworthstone/frankenpandas/commit/e5557ec453f8da195223601372824f5ed5b67ccc))
- **pd.to_timedelta()** and `timedelta_total_seconds()` ([`885bac2`](https://github.com/Dicklesworthstone/frankenpandas/commit/885bac292e34885981cd63170130a05278bd4b8a))

#### Added -- Index
- **MultiIndex.reorder_levels()** for hierarchical index level reordering ([`11eb0b7`](https://github.com/Dicklesworthstone/frankenpandas/commit/11eb0b7a7f0d3630e5872d8d92e0e0e135dc4bd0))

#### Added -- Workspace
- **Top-level `frankenpandas` facade crate** with unified public API (`use frankenpandas::prelude::*`) re-exporting all subsystem types ([`74795b4`](https://github.com/Dicklesworthstone/frankenpandas/commit/74795b4cfaa30d473ed205415b67415838bad8bd))

#### Fixed
- JSONL reader now unions all keys across rows for ragged-schema support ([`d5b4e24`](https://github.com/Dicklesworthstone/frankenpandas/commit/d5b4e2447ce2e51918493167878211888d9d3700))
- Negative timedelta round-trip bug in parse_hms ([`89d81ab`](https://github.com/Dicklesworthstone/frankenpandas/commit/89d81ab664228a5c6e9df0d2bde21b3b57fc05a8))
- Add error types, NullKind, and Column to facade public API ([`c9d812f`](https://github.com/Dicklesworthstone/frankenpandas/commit/c9d812f31549d0593038bbbd22564541b785ea72))

#### Tests
- End-to-end pandas workflow integration tests ([`dd20bae`](https://github.com/Dicklesworthstone/frankenpandas/commit/dd20bae717caa109c9e19e286e657e42a3b4f924))

---

### 2026-03-15 -- SQL/Excel/Feather I/O, MultiIndex Foundation, Categorical Dtype

#### Added -- I/O (3 new formats)
- **SQL (SQLite) I/O** via rusqlite -- `read_sql` / `to_sql` ([`df38cff`](https://github.com/Dicklesworthstone/frankenpandas/commit/df38cffa052c038ae5f660fbe0ed1dfd122f8df2))
- **Excel I/O** with property-based fuzz tests ([`49aa1b6`](https://github.com/Dicklesworthstone/frankenpandas/commit/49aa1b63d3ca7c6e22d06be474fd12a4c27358bf))
- **Arrow IPC / Feather v2** read/write ([`0fa35b2`](https://github.com/Dicklesworthstone/frankenpandas/commit/0fa35b299431e523e9e77e5f12458816f1ddb72c), [`bab40f8`](https://github.com/Dicklesworthstone/frankenpandas/commit/bab40f83ce650be49334df261244c415fab5d9d2))

#### Added -- Frame
- **Categorical dtype** support as Series metadata layer ([`7a384c5`](https://github.com/Dicklesworthstone/frankenpandas/commit/7a384c5c254bda2646615ba0b16374abfe7e1135))
- MultiIndex integrated with DataFrame `set_index` / `reset_index` ([`17777c9`](https://github.com/Dicklesworthstone/frankenpandas/commit/17777c939d89ae7e6513773681bf8604c660effd))
- `to_dict` tight orient, `to_series_dict`, label display fixes ([`4872463`](https://github.com/Dicklesworthstone/frankenpandas/commit/487246313c0b3dab55f35a30d06f88c96a63aa97))

#### Added -- Index
- **MultiIndex foundation** for hierarchical indexing ([`80e8d00`](https://github.com/Dicklesworthstone/frankenpandas/commit/80e8d00eac216dd7874b579d6cad44f6dcebefcf))

#### Added -- Conformance
- DataFrame arithmetic/comparison property tests ([`3ba5d5d`](https://github.com/Dicklesworthstone/frankenpandas/commit/3ba5d5d64b5d9ee6619c6bf7b559a171b3348358))
- Join/filter performance baselines ([`fc42c5e`](https://github.com/Dicklesworthstone/frankenpandas/commit/fc42c5e4d278be909824eb5a226207e3a8d293b1))
- Property-based IO round-trip tests for SQL, Excel, Feather ([`0b44cbe`](https://github.com/Dicklesworthstone/frankenpandas/commit/0b44cbe2f814c35fca653f8723728c9eb7593907))
- 15 adversarial parser tests for CSV/JSON/SQL ([`4835189`](https://github.com/Dicklesworthstone/frankenpandas/commit/4835189a0b0350139332b36b0e1c4a02260f3028))
- eval/query expression parity tests ([`6a9a854`](https://github.com/Dicklesworthstone/frankenpandas/commit/6a9a85433d3ecbc6d1d4140d95c8c235fe62ea11))
- merge_asof edge-case tests ([`c853e97`](https://github.com/Dicklesworthstone/frankenpandas/commit/c853e9755d2fa0a8403becb7430b6fa44cdcce00))

#### Fixed
- `to_multi_index` handles Float64/Bool columns correctly ([`dc8cf48`](https://github.com/Dicklesworthstone/frankenpandas/commit/dc8cf4805dbbe9fbde34a73db5a2dfc4c97184e2))
- Reject empty SQL table names in validation ([`82e24e9`](https://github.com/Dicklesworthstone/frankenpandas/commit/82e24e91c5d7d8bd5e3fb6e15d26ed7c9ce5202d))
- 3 bugs found in IO/conformance code review ([`5f0dd0b`](https://github.com/Dicklesworthstone/frankenpandas/commit/5f0dd0b7e2f64e1af2581151a2c4fa324b7e6b8f))

---

### 2026-03-12 -- Timezone Handling Rewrite, JSON Orient Parity, from_records

#### Added
- **chrono-tz support** and complete timezone handling rewrite ([`816519b`](https://github.com/Dicklesworthstone/frankenpandas/commit/816519b0822e2933a7a6e3a6fccd7eac8d6f10df))
- Series-level ambiguous/nonexistent timezone localization policies ([`add04cc`](https://github.com/Dicklesworthstone/frankenpandas/commit/add04cc116029095ff8bdff02cc49fd3c97cf024))
- **DataFrame.from_records** row-record constructor (matrix_rows) ([`ec14b61`](https://github.com/Dicklesworthstone/frankenpandas/commit/ec14b6142a5c3bf07b842305354fbe80ed6f51de))
- DataFrame `to_json` orient parity with pandas ([`e1352d9`](https://github.com/Dicklesworthstone/frankenpandas/commit/e1352d9fcc6fe97b9bc743bc54f10e58f2606995))

#### Fixed
- Align JSON orients with pandas semantics ([`9f46f2d`](https://github.com/Dicklesworthstone/frankenpandas/commit/9f46f2da5cca00ba3d518f31bb1dbb81416c3bff))

---

### 2026-03-10 -- Expression Engine Local Variables

#### Added
- **@local variable bindings** in expression engine for `eval()` / `query()`, expanding DataFrame operations ([`49a14c2`](https://github.com/Dicklesworthstone/frankenpandas/commit/49a14c244ed00ffd23ec8c70078111f3c7f2423d))

---

### 2026-03-03 -- 2026-03-04 -- Massive Frame API Expansion

This two-day sprint added approximately 60 new DataFrame/Series methods covering statistics, window functions, reshaping, accessors, and groupby extensions.

#### Added -- DataFrame
- `combine` for element-wise binary operations ([`34e8a35`](https://github.com/Dicklesworthstone/frankenpandas/commit/34e8a359ce79a90441134ee4568c0481ebfeda18))
- `std_agg_ddof`, `var_agg_ddof` for configurable degrees of freedom ([`f584f56`](https://github.com/Dicklesworthstone/frankenpandas/commit/f584f5690fcf83988f0784ae387a05744eb806d0))
- `corrwith_axis`, `map_with_default` ([`5a2bc08`](https://github.com/Dicklesworthstone/frankenpandas/commit/5a2bc08640ecf20e8a0fd3263386f25a2c7b099f))
- `corr_min_periods`, `cov_min_periods` ([`bdc49e8`](https://github.com/Dicklesworthstone/frankenpandas/commit/bdc49e8de25e6974c63eb3514a6bb833c53e710a))
- `fill_arithmetic` (radd/rsub/rmul/rdiv), `extract_to_frame`, `margins_name` ([`a69d85e`](https://github.com/Dicklesworthstone/frankenpandas/commit/a69d85efebac24dda8724214535d23759162dd64))
- `rename_index` ([`ef41f54`](https://github.com/Dicklesworthstone/frankenpandas/commit/ef41f54cdfbc64a26d055e5d046a728ead60f690))
- `to_csv_options` ([`d520b8f`](https://github.com/Dicklesworthstone/frankenpandas/commit/d520b8faca28b28202fcdfc6b79a713fc46e0008))
- `pivot_table_multi_agg` ([`64eacff`](https://github.com/Dicklesworthstone/frankenpandas/commit/64eacffd6524f4ce4bba4de4b9d92d3fc185624c))
- `pivot_table_with_margins`, `interpolate_method`, `fillna_dict` ([`ed701c0`](https://github.com/Dicklesworthstone/frankenpandas/commit/ed701c0e71a47dce63685a22b6ec0aae01a2fe77))
- `pivot_table_fill`, `count_na`, `swaplevel` ([`aa6e20b`](https://github.com/Dicklesworthstone/frankenpandas/commit/aa6e20bd2f7723992f558a731af1c010a1559cd5))
- `pivot_table_multi_values` ([`1fcb08f`](https://github.com/Dicklesworthstone/frankenpandas/commit/1fcb08f1f7fa8af752bc8524295adf721dccf431))
- `reindex_with_method`, groupby `sort` parameter ([`ba24056`](https://github.com/Dicklesworthstone/frankenpandas/commit/ba24056fb4bee776a442913bacaa6a4153e6f1d8))
- `assign_fn`, `applymap_na_action`, `isin_dict` ([`c782fc7`](https://github.com/Dicklesworthstone/frankenpandas/commit/c782fc78b33a2e7f952e21b8efcc65ae94b710f3))
- `na_position` control, `ignore_index` concat, `replace_dict`, groupby `agg_multi` ([`631b3f7`](https://github.com/Dicklesworthstone/frankenpandas/commit/631b3f74e2bc430bacfbf915495aba554d1e01e2))
- `loc_bool_series` and `iloc_bool_series` accessors ([`edf58dc`](https://github.com/Dicklesworthstone/frankenpandas/commit/edf58dc1f80ac3b0f15c2f3722cf8d6c19250140))
- `loc`/`iloc` boolean mask, slice, and row accessors; groupby `as_index=false` ([`e28065e`](https://github.com/Dicklesworthstone/frankenpandas/commit/e28065e59ef73a17aea7f057af0cddce22095618))
- `pow_df` ([`7337bba`](https://github.com/Dicklesworthstone/frankenpandas/commit/7337bbae7c1a13418f3e2f0007bd20ec4155a21a))
- `drop_columns`, `clip_with_series`, `nlargest_multi` ([`86b31a2`](https://github.com/Dicklesworthstone/frankenpandas/commit/86b31a24e5a50fca5914e41b2e911e3bf1b465f8))
- `reindex_columns`, `from_tuples` ([`9ba17d7`](https://github.com/Dicklesworthstone/frankenpandas/commit/9ba17d7b9b8cc8fc5ff8ccb1e1061fa76c944be5))
- `reindex_fill`, rolling center mode ([`1fcb08f`](https://github.com/Dicklesworthstone/frankenpandas/commit/1fcb08f1f7fa8af752bc8524295adf721dccf431))
- `truncate`, `to_string_truncated` ([`bf805b0`](https://github.com/Dicklesworthstone/frankenpandas/commit/bf805b0ae8983056899575e759cc430bc8ab609c))
- `where_cond_series`, `mask_series`, `fillna_limit` ([`1b0d699`](https://github.com/Dicklesworthstone/frankenpandas/commit/1b0d699310dbcda42d346f3985ac5e8b1e29bdca))
- `astype`, `astype_safe`, `rolling_with_center` ([`ad90b46`](https://github.com/Dicklesworthstone/frankenpandas/commit/ad90b46f12326635a4c66fa2ed49619c3719a2f4))
- `from_tuples_with_index`, timezone operations ([`92d599c`](https://github.com/Dicklesworthstone/frankenpandas/commit/92d599c42c7e1b2c7ab8d4d6b75b41c1ff83b8eb))
- `skipna` aggregates, `first`/`last` offset, date arithmetic helpers ([`8cebdeb`](https://github.com/Dicklesworthstone/frankenpandas/commit/8cebdeb64b5ef0865adae1b3af28465617a3f458))

#### Added -- Series
- `var_ddof`, `std_ddof` for configurable degrees of freedom ([`d5b6aa6`](https://github.com/Dicklesworthstone/frankenpandas/commit/d5b6aa6cd242330133c982ebb5582546b0e3972f))
- `duplicated_keep` with `DuplicateKeep` parameter ([`809c243`](https://github.com/Dicklesworthstone/frankenpandas/commit/809c243de94f73c2aa45c5d228e3949061148e0d))
- `value_counts_bins` for binned frequency counting ([`93d042e`](https://github.com/Dicklesworthstone/frankenpandas/commit/93d042e78c174ea79ae59f8568b8fcfc42ec091e))
- `describe`, `drop_duplicates_keep` ([`d520b8f`](https://github.com/Dicklesworthstone/frankenpandas/commit/d520b8faca28b28202fcdfc6b79a713fc46e0008))
- `agg()` multi-function aggregation, `groupby()` ([`6b7a610`](https://github.com/Dicklesworthstone/frankenpandas/commit/6b7a61003c5b632fdca22fe5f8550a4aa7ceeaca), [`7337bba`](https://github.com/Dicklesworthstone/frankenpandas/commit/7337bbae7c1a13418f3e2f0007bd20ec4155a21a))
- `item()` ([`ef41f54`](https://github.com/Dicklesworthstone/frankenpandas/commit/ef41f54cdfbc64a26d055e5d046a728ead60f690))
- `replace_regex` ([`fc0f0a6`](https://github.com/Dicklesworthstone/frankenpandas/commit/fc0f0a6c7e8944883411d9a85e5e185a1a0d73af))
- `slice_replace`, `describe_dtypes`, `value_counts_subset` ([`ccd59f4`](https://github.com/Dicklesworthstone/frankenpandas/commit/ccd59f4e90265e130097f69f7d5fe67813a7989e))
- `replace_map`, `apply_fn_na` ([`70c2196`](https://github.com/Dicklesworthstone/frankenpandas/commit/70c21965c844adbcdd250cefa04f39651862b473))

#### Added -- Window Functions
- Rolling `skew`/`kurt` ([`ef41f54`](https://github.com/Dicklesworthstone/frankenpandas/commit/ef41f54cdfbc64a26d055e5d046a728ead60f690))
- Expanding `skew`/`kurt` ([`f7d419c`](https://github.com/Dicklesworthstone/frankenpandas/commit/f7d419caf5eae30214b2d6c15df406371cba586f))
- `to_records` and fill arithmetic ([`753ea6c`](https://github.com/Dicklesworthstone/frankenpandas/commit/753ea6c21d8e18ad5dbb3c6ec709152e3ed25d38))
- `rolling.agg`, `to_numpy_2d`, `dt.to_timestamp` ([`bc27b5f`](https://github.com/Dicklesworthstone/frankenpandas/commit/bc27b5f53b6bdea260ab814867a50c3bdf9c7f68))

#### Added -- Datetime Accessor
- `dt.ceil`, `dt.floor` ([`82229b0`](https://github.com/Dicklesworthstone/frankenpandas/commit/82229b0ebf2a7d6a76bb1768e70e732fc2771dd4))
- `dt.round` ([`fc0f0a6`](https://github.com/Dicklesworthstone/frankenpandas/commit/fc0f0a6c7e8944883411d9a85e5e185a1a0d73af))

#### Added -- GroupBy
- `SeriesGroupBy.agg`, `var`, `median`, `prod` ([`fc0f0a6`](https://github.com/Dicklesworthstone/frankenpandas/commit/fc0f0a6c7e8944883411d9a85e5e185a1a0d73af), [`9ba17d7`](https://github.com/Dicklesworthstone/frankenpandas/commit/9ba17d7b9b8cc8fc5ff8ccb1e1061fa76c944be5))
- Groupby `dropna` parameter, `sample_weights` ([`82229b0`](https://github.com/Dicklesworthstone/frankenpandas/commit/82229b0ebf2a7d6a76bb1768e70e732fc2771dd4))
- Resample multi-aggregate ([`70c2196`](https://github.com/Dicklesworthstone/frankenpandas/commit/70c21965c844adbcdd250cefa04f39651862b473))

#### Added -- Index
- `Index.name`, `map_series`, `names`, `to_flat_index` ([`59374a2`](https://github.com/Dicklesworthstone/frankenpandas/commit/59374a2ee11587f1a48651de8e69fdb43cf7d50b), [`ad90b46`](https://github.com/Dicklesworthstone/frankenpandas/commit/ad90b46f12326635a4c66fa2ed49619c3719a2f4))

#### Added -- I/O
- Headerless CSV input with auto-generated column names ([`2694498`](https://github.com/Dicklesworthstone/frankenpandas/commit/2694498db8bbdbc495df320bda93a7b9489a3a9d))

#### Added -- Conformance
- `--python-bin` CLI flag for conformance runner, join benchmark binary ([`45bb12b`](https://github.com/Dicklesworthstone/frankenpandas/commit/45bb12b8ae3f361fe3ab7236984fdd257978ed79))

#### Fixed
- `describe()` type mismatch for string column statistics ([`dd638ad`](https://github.com/Dicklesworthstone/frankenpandas/commit/dd638ade448eaa394186b680d9d610bb6d83045a))
- Explicit dereference in `max_by_key` closure ([`3b22069`](https://github.com/Dicklesworthstone/frankenpandas/commit/3b2206959945b70be4adfbd51fe632a190996a5a))
- Clippy warnings in `interpolate_method` and `pivot_table_with_margins` ([`a030483`](https://github.com/Dicklesworthstone/frankenpandas/commit/a030483458fa3eee3bb405d1d25ac26f35079bc4))
- Explicit Scalar constructors in isin_dict and applymap tests ([`f7a0784`](https://github.com/Dicklesworthstone/frankenpandas/commit/f7a078404245292cc41bd06c91a10d276e63082c))

---

### 2026-03-02 -- GroupBy Observability, Bool Label Formatting

#### Added
- Runtime admission policy observability wired into groupby operations ([`ff6128d`](https://github.com/Dicklesworthstone/frankenpandas/commit/ff6128d3a488ad1c2d49d56fb90c027d51adbed0))

#### Fixed
- Format bool groupby key labels as pandas-style `"True"` / `"False"` ([`44dab10`](https://github.com/Dicklesworthstone/frankenpandas/commit/44dab10a14b084890d63545222db789f447ab89d))

#### Chore
- Normalize rustfmt formatting across fp-expr, fp-frame, fp-groupby, fp-io, fp-join ([`23be4e3`](https://github.com/Dicklesworthstone/frankenpandas/commit/23be4e37103671a9063e467982da945318b66e26))
- Comprehensive `from_csv_with_options` test coverage ([`5152a40`](https://github.com/Dicklesworthstone/frankenpandas/commit/5152a4038ad719feafb89e5e053b21453a1bba8e))

---

### 2026-02-25 -- 2026-02-26 -- API Surface Explosion: 100+ Methods

This phase represents the largest single expansion of the pandas API surface, adding approximately 100 new methods across Series, DataFrame, GroupBy, and I/O.

#### Added -- DataFrame
- Major frame implementation rewrite and expression engine integration ([`ae437eb`](https://github.com/Dicklesworthstone/frankenpandas/commit/ae437eb70b7d0feef793427a17f25179ccb55637))
- `where`, `mask`, `iterrows`, `itertuples`, `items`, `assign`, `pipe` ([`c23f3b3`](https://github.com/Dicklesworthstone/frankenpandas/commit/c23f3b366df374e8f4eb17e405f3f075e858cc9c))
- `rank`, `melt`, `pivot_table` ([`158b546`](https://github.com/Dicklesworthstone/frankenpandas/commit/158b54669dff4a1ee7e9335d9f30a7fa89f229e5))
- `agg`, `applymap`, `transform`, `corr`, `cov`, `nlargest`, `nsmallest`, `reindex` ([`4dc605f`](https://github.com/Dicklesworthstone/frankenpandas/commit/4dc605fa069ef7f66cb7c5a219242657aad2a223))
- `groupby` integration, `sample`, `info` ([`0034f3d`](https://github.com/Dicklesworthstone/frankenpandas/commit/0034f3d1a586b581c7a28e441f6a094d2419baee))
- `stack` / `unstack`, closure-based `apply_fn`, `map_values` ([`49c7417`](https://github.com/Dicklesworthstone/frankenpandas/commit/49c7417db3f25b6aab6c609f563b9091abd663fd))
- Row-wise `var`/`std` in `DataFrame.apply`, refined `clip` ([`2f7f5a4`](https://github.com/Dicklesworthstone/frankenpandas/commit/2f7f5a45153e68c7abdd1dce3069ecc17cde1591))
- Row-wise `median` in `DataFrame.apply`, typed Scalar returns ([`ee48142`](https://github.com/Dicklesworthstone/frankenpandas/commit/ee481424ea601b1c22bcfa2da0f2ed71d650af87))

#### Added -- Series
- `str` accessor with 15 string operations ([`e2224bf`](https://github.com/Dicklesworthstone/frankenpandas/commit/e2224bf386d75e0b29eda19ecb7ec70edc9f4bc8))
- `idxmin`, `idxmax`, `nlargest`, `nsmallest`, `pct_change` ([`edba19e`](https://github.com/Dicklesworthstone/frankenpandas/commit/edba19ec4da03004cde3b6a0d93b098abeaa654d))
- `where_cond`, `mask`, `isin`, `between` ([`ddf8726`](https://github.com/Dicklesworthstone/frankenpandas/commit/ddf87260109b28e99cba825096c512724f7bd2a6))
- `convert_dtypes`, `map_with_na_action`, `case_when` ([`0b7bc94`](https://github.com/Dicklesworthstone/frankenpandas/commit/0b7bc94d8ffd2ad39e00a36d362e82b179bc74b8))
- Comparison ops, `get_dummies` ([`15393ef`](https://github.com/Dicklesworthstone/frankenpandas/commit/15393ef2fa121f2afdb474ecf1c55d7de1bd96a4))
- `dt` accessor (DatetimeAccessor), element-wise ops, merge trait, string methods ([`d3fb430`](https://github.com/Dicklesworthstone/frankenpandas/commit/d3fb430a981d9bf0a3d8a84942dfa047f0a6247c))
- 15 new Series methods, 4 new DataFrame methods, groupby/types conformance fixes ([`22eb92a`](https://github.com/Dicklesworthstone/frankenpandas/commit/22eb92a3b8566afe619cbd614378d00967d95a0e))

#### Added -- Window Functions
- `rolling` / `expanding` window operations for Series ([`e82e531`](https://github.com/Dicklesworthstone/frankenpandas/commit/e82e5315d9f4a7667472d9340fdaa99889e0f6c7))
- Rolling `corr`/`cov` ([`15393ef`](https://github.com/Dicklesworthstone/frankenpandas/commit/15393ef2fa121f2afdb474ecf1c55d7de1bd96a4))
- **EWM (exponentially weighted) windows** ([`c41d37d`](https://github.com/Dicklesworthstone/frankenpandas/commit/c41d37d0bc6f2d739eaa806538fe73038a838521))

#### Added -- GroupBy
- `nunique`, `prod`, `size` aggregation functions ([`371d638`](https://github.com/Dicklesworthstone/frankenpandas/commit/371d638e6042e4e36ff6231aca8036c6289eb073))
- **merge_asof** ([`c41d37d`](https://github.com/Dicklesworthstone/frankenpandas/commit/c41d37d0bc6f2d739eaa806538fe73038a838521))

#### Added -- I/O
- **Parquet read/write** via Arrow RecordBatch integration ([`262235b`](https://github.com/Dicklesworthstone/frankenpandas/commit/262235b14594bf21e586841ad423507439da6d69))

#### Fixed
- Filter non-numeric columns in `corr`/`cov` pairwise_stat ([`26e5e7e`](https://github.com/Dicklesworthstone/frankenpandas/commit/26e5e7e9e28bba1df5a69b1684f41c68cd4aa3f0))
- `Column::from_values` in applymap, groupby count test ([`c31c3d8`](https://github.com/Dicklesworthstone/frankenpandas/commit/c31c3d881944d89a201026fc0bf7f825ba699e08))
- Dead branches in `pct_change` and `rolling_count` ([`531cfb6`](https://github.com/Dicklesworthstone/frankenpandas/commit/531cfb655403aaed746636940f328f7799818cb9))
- Bool variant in debug column name parser ([`607ab58`](https://github.com/Dicklesworthstone/frankenpandas/commit/607ab582ce3501d6e74e7f0a2a454a233c443e16))
- DataFrame arithmetic ops, type coercion, label width, diff comparison ([`3ba9008`](https://github.com/Dicklesworthstone/frankenpandas/commit/3ba900865bf8aa28222b4853c913acfe0977c5f9))
- Exactly-representable float in JSON round-trip test ([`828c0ab`](https://github.com/Dicklesworthstone/frankenpandas/commit/828c0abe18a648fae19a46cb1b24dd1958f6226e))

---

### 2026-02-20 -- 2026-02-21 -- Correctness Fixes and Series Value Operations

#### Added
- Series `value_counts`, `sort`, `tail`, `isna`/`notna`, `dropna`/`fillna`, `count`, `isnull`/`notnull` ([`f1316f2`](https://github.com/Dicklesworthstone/frankenpandas/commit/f1316f2a8f974057adc3defcc199259eb5f0fbc9))
- Pandas oracle fixture generator and new conformance test cases ([`187ebab`](https://github.com/Dicklesworthstone/frankenpandas/commit/187ebab9b6ad2f2321d55d649b5d5441adb37a8f))
- FP-P2D-053 set/reset_index, FP-P2D-054 duplicate detection, FP-P2D-055 series arithmetic conformance packets ([`6f2180c`](https://github.com/Dicklesworthstone/frankenpandas/commit/6f2180c59bef60c8a5d31a2f72496f377c9d5c2a), [`fda6a0a`](https://github.com/Dicklesworthstone/frankenpandas/commit/fda6a0af057162c2d25ea7c639b0e2ee54458a29))

#### Fixed
- Preserve column order and dtype through subsetting, fix outer join performance, add scalar-to-string casting, correct numeric boundaries ([`d3b1542`](https://github.com/Dicklesworthstone/frankenpandas/commit/d3b1542517942673685229b689f059d024be673c))
- Rewrite expression linearity detection, refactor groupby to preserve Scalar types, fix JSON/recovery edge cases ([`17b6fef`](https://github.com/Dicklesworthstone/frankenpandas/commit/17b6fef7a90bde469f303163701664ea9f6cd5f2))
- Char boundary panic in governance gate, float precision in join keys, underflow in conformal guard ([`54cef39`](https://github.com/Dicklesworthstone/frankenpandas/commit/54cef3982d84068f232ac02638ed0531d0c5cbcc))
- HyperLogLog rho calculation: use sentinel bit instead of unconditional OR ([`e39bc95`](https://github.com/Dicklesworthstone/frankenpandas/commit/e39bc95e9167bef7d1daaf4058bded19c67901aa))

---

### 2026-02-19 -- Expression Engine, Null Handling, DataFrame Parity APIs

#### Added -- Expression Engine (fp-expr)
- Comparison expression operators and delta evaluation ([`626267d`](https://github.com/Dicklesworthstone/frankenpandas/commit/626267d267077368c190dc6e4bdb16a946823404))
- DataFrame-backed evaluation bridge via `EvalContext` ([`85ecf53`](https://github.com/Dicklesworthstone/frankenpandas/commit/85ecf53b30e16355596caae67ef938641f3b3d00))
- Boolean mask composition operators and planner logical ops ([`03fc784`](https://github.com/Dicklesworthstone/frankenpandas/commit/03fc784b5f4c40821dea314f1dbd7473c74d30c3))
- DataFrame `query()`-style filter helper ([`58cd95a`](https://github.com/Dicklesworthstone/frankenpandas/commit/58cd95afaa732b8afd47136cb37ee5928efb4f5a))

#### Added -- Frame
- `fillna` / `dropna` null-cleaning primitives ([`555a8a9`](https://github.com/Dicklesworthstone/frankenpandas/commit/555a8a9d15848cc14f34263ae3b3cb0d21f1d71a))
- `isna` / `notna` null-mask APIs for Series and DataFrame ([`9d27dda`](https://github.com/Dicklesworthstone/frankenpandas/commit/9d27ddabe203fd8cf6da18b9616d418f68f354f5))
- `dropna` with `how`/`subset` options ([`9e9090b`](https://github.com/Dicklesworthstone/frankenpandas/commit/9e9090b8cd0a4edfde8c3d458d9c04d328c354a9))
- `dropna` axis=1 options ([`1e020cd`](https://github.com/Dicklesworthstone/frankenpandas/commit/1e020cd8ec9d0f47589a1bf234db0654fada96ff))
- pandas `isnull`/`notnull` aliases, axis=1 dropna default ([`364077f`](https://github.com/Dicklesworthstone/frankenpandas/commit/364077f27091357c16a700212a4b418626f26d97))
- `dropna` thresh semantics for rows and columns ([`8a3a971`](https://github.com/Dicklesworthstone/frankenpandas/commit/8a3a97151b3c3d4ff75f3e59df3071d30240a081))
- Series `head`/`tail` API parity ([`dae33f3`](https://github.com/Dicklesworthstone/frankenpandas/commit/dae33f3c634b984e87cccf9b4e14db9e4366f732))
- DataFrame `set_index` / `reset_index` parity ([`d005c31`](https://github.com/Dicklesworthstone/frankenpandas/commit/d005c311e164cac7409df9faa954f369be19672b))
- Series/DataFrame `astype` coercion APIs ([`31fa57e`](https://github.com/Dicklesworthstone/frankenpandas/commit/31fa57efb11a2316e45a19ac8b8bad3a6e01e38f))
- `duplicated` / `drop_duplicates` DataFrame parity ([`738d61b`](https://github.com/Dicklesworthstone/frankenpandas/commit/738d61b7d0b9ce22f059c6b05a2e113691209f52))
- DataFrame scalar-broadcast constructor parity ([`c657542`](https://github.com/Dicklesworthstone/frankenpandas/commit/c657542569013492b8ce5d68de4737b4fc231ea2))
- Multi-column `astype` mapping parity ([`8d53b84`](https://github.com/Dicklesworthstone/frankenpandas/commit/8d53b8425a2f5023c016f3b41d133e1f10c33135))
- Series `any`/`all` aggregation and DataFrame `sort_index`/`sort_values` parity ([`34dffda`](https://github.com/Dicklesworthstone/frankenpandas/commit/34dffda62079313f6fb00b76ca639a7aff483d75))
- Duplicate index label support when all inputs are exactly aligned ([`3f028b6`](https://github.com/Dicklesworthstone/frankenpandas/commit/3f028b6f155c8f0d90518bebc2940d9280f6eea6))
- Mixed Int64/Utf8 index labels in `reset_index` ([`82e9fbb`](https://github.com/Dicklesworthstone/frankenpandas/commit/82e9fbb7d3a283e3d06ddb0eb1b63d07a224dcfb))

#### Added -- I/O
- JSON orient=index read/write path ([`3cda217`](https://github.com/Dicklesworthstone/frankenpandas/commit/3cda217e440d79d58140e4a5590e9cb2f84644c6))
- File-path CSV options reader ([`d61d610`](https://github.com/Dicklesworthstone/frankenpandas/commit/d61d61064b63c5c5cc524ae2e1c3f36b79d57363))
- Preserve split-orient JSON index labels on roundtrip ([`e324230`](https://github.com/Dicklesworthstone/frankenpandas/commit/e324230d40ce5f628038cfad4d1f26a89d247b4e))

#### Tests
- FP-P2D-040 sort and FP-P2D-041 any/all oracle-backed conformance packets ([`f41e0b0`](https://github.com/Dicklesworthstone/frankenpandas/commit/f41e0b0b65e6259545e06e67264877b5db6a515c))

---

### 2026-02-18 -- Merge/Join Parity, Expression Operators, Licensing

#### Added -- Join (fp-join)
- **Cross join** semantics ([`8037afa`](https://github.com/Dicklesworthstone/frankenpandas/commit/8037afa227a42d5be84b94e244293cca2c803a1a))
- Merge `indicator` parity (pandas `_merge` column) ([`d71abd5`](https://github.com/Dicklesworthstone/frankenpandas/commit/d71abd5f1e69afb195020779e69a31a31db91475))
- Merge `validate` parity ([`c0007a0`](https://github.com/Dicklesworthstone/frankenpandas/commit/c0007a088003724939c234119f98e43134b6e55a))
- Merge `suffixes` tuple parity and collision semantics ([`56ffb80`](https://github.com/Dicklesworthstone/frankenpandas/commit/56ffb80ba5e1035ac2dd329279ce43278372b3e9))
- Merge `sort` flag parity ([`14da9af`](https://github.com/Dicklesworthstone/frankenpandas/commit/14da9af669c1d05e6f5943b085e8e333a1e89e40))
- Reject series_join cross, negative packet coverage ([`597cffc`](https://github.com/Dicklesworthstone/frankenpandas/commit/597cffc19b9c9fc06856e4d5a8b46ee6ce665b0b))

#### Added -- Expression Engine (fp-expr)
- `Sub`/`Mul`/`Div` expression operators ([`f389846`](https://github.com/Dicklesworthstone/frankenpandas/commit/f3898463ca834baa0e5f77b6e673c19d998d1fda))

#### Added -- Frame
- DataFrame `sort_index` / `sort_values` core APIs ([`8024fca`](https://github.com/Dicklesworthstone/frankenpandas/commit/8024fcaa556c1744d2b009eadfe1343f590ab461))

#### Added -- Conformance
- FP-P2D-028 through FP-P2D-039: DataFrame concat axis=0/1, merge cross-join, sort, suffix, validate, indicator packets ([`48597143`](https://github.com/Dicklesworthstone/frankenpandas/commit/48597143f8d2f4fbfd18e57822c73a52f2ea378c)...[`6ccbc1a`](https://github.com/Dicklesworthstone/frankenpandas/commit/6ccbc1ab9388ee7db2c14dc2caef5a6bc79acb8a))

#### Changed
- **MIT + OpenAI/Anthropic rider** adopted across workspace and all crates ([`a9889ca`](https://github.com/Dicklesworthstone/frankenpandas/commit/a9889cafc70ec04293d907d06f0d80868263e4e8))

---

### 2026-02-17 -- Conformance Packet Expansion (FP-P2D-014 through FP-P2D-027)

#### Added
- Dynamic conformance packet-coverage tests ([`ebf9e0f`](https://github.com/Dicklesworthstone/frankenpandas/commit/ebf9e0f41667a0fcb3a8e669aa02e2c430e0939f))
- Negative `iloc` positions and groupby `std`/`var`/`median` operations ([`c6fef8d`](https://github.com/Dicklesworthstone/frankenpandas/commit/c6fef8d50e75f0ce5b67e32fe1ce86488d0423d8))
- FP-P2D-014/015/016: DataFrame merge/concat, NaN aggregation, CSV round-trip packets ([`1aa7aba`](https://github.com/Dicklesworthstone/frankenpandas/commit/1aa7aba7e7b2fe912cdb4815765f3107c3d51e9a))
- FP-P2D-017 through 021: Series/DataFrame constructor path packets ([`9fcb157`](https://github.com/Dicklesworthstone/frankenpandas/commit/9fcb1572af27883b44a0daef106ff554d6c96686))
- FP-P2D-022: DataFrame constructor list-like shape taxonomy ([`bdc107f`](https://github.com/Dicklesworthstone/frankenpandas/commit/bdc107ff11f88f9723bdd951b65134586f87199c))
- FP-P2D-023: constructor dtype/copy parity ([`bd909fd`](https://github.com/Dicklesworthstone/frankenpandas/commit/bd909fdf7b20f49af965570b2e4990c5481c34bc))
- FP-P2D-024: dtype-spec normalization ([`7aef5b6`](https://github.com/Dicklesworthstone/frankenpandas/commit/7aef5b6a0acbd8bad2d05331d2d9e67718077ca0))
- DataFrame selector and head-tail parity packets ([`8d8993b`](https://github.com/Dicklesworthstone/frankenpandas/commit/8d8993bb1cabeeb6b92e65b1dfceef44e16e6f07))
- FP-P2D-027: negative-n head/tail coverage ([`7477ad7`](https://github.com/Dicklesworthstone/frankenpandas/commit/7477ad7c3e0342d5d45ead4cb7fd8e1f78f1a072))
- SeriesConstructor and DataFrameFromSeries conformance fixture operations ([`e872cc9`](https://github.com/Dicklesworthstone/frankenpandas/commit/e872cc9df9f278e39b8a1912b7f2bd3950c46d02))
- FP-P2C-010/011 conformance packets for iloc, loc, filter, groupby std/var/median ([`962d9d6`](https://github.com/Dicklesworthstone/frankenpandas/commit/962d9d6a487a7812e3267c78986f045a150459d6))

---

### 2026-02-16 -- DataFrame loc/iloc, Conformance Hardening, Runtime Governance

#### Added -- Frame
- DataFrame `loc`/`iloc` label-based and positional row selection ([`ccd3187`](https://github.com/Dicklesworthstone/frankenpandas/commit/ccd3187887622b5a105ac9a8e00cc767538109d5), [`5ab540e`](https://github.com/Dicklesworthstone/frankenpandas/commit/5ab540e1cc4cdf4913a27aeca74a3f1ade65f0a7))
- Boolean mask type validation and error-path conformance support ([`12d7de2`](https://github.com/Dicklesworthstone/frankenpandas/commit/12d7de26836bf06d42f9d06e9cf76022e681b36d))
- `MissingIndexColumn` error variant and CSV edge-case tests ([`d3b09c0`](https://github.com/Dicklesworthstone/frankenpandas/commit/d3b09c0358969a900478f039dbd944305b1c55d9))

#### Added -- Conformance
- Fall back to fixture expectations when live oracle is unavailable ([`c29188b`](https://github.com/Dicklesworthstone/frankenpandas/commit/c29188bbcae1a0d2629bb9644b8a217c22530854))
- Capture oracle errors as report mismatches instead of propagating ([`90bffe2`](https://github.com/Dicklesworthstone/frankenpandas/commit/90bffe2c5bf28cd1f4ee688274ab9c838fc33854))
- Harden pandas import fallback, add `series_filter`/`series_head` ops ([`9a9535c`](https://github.com/Dicklesworthstone/frankenpandas/commit/9a9535c549f9c092f2fd14a758776a75e89e8a2c))
- Enforce sidecar `artifact_id` packet coherence ([`4308f4b`](https://github.com/Dicklesworthstone/frankenpandas/commit/4308f4bc4895af14f736e7206b1ec43d7fab1f69))

#### Fixed
- Reject sidecars with envelope/packet count mismatches ([`30f780f`](https://github.com/Dicklesworthstone/frankenpandas/commit/30f780faf199a62829d62a53ebf8e44a76e9483c))
- RaptorQ placeholder scrub status changed from "ok" to "placeholder" ([`b675bd5`](https://github.com/Dicklesworthstone/frankenpandas/commit/b675bd5172c610b1b7b1c04e319fd85beebf492a))
- Cap decode proofs and enforce sidecar bounds ([`2eb9729`](https://github.com/Dicklesworthstone/frankenpandas/commit/2eb97298106c87664b21d7d93ea3a02f1b317139))

---

### 2026-02-15 -- Series loc/iloc, FrankenTUI Scaffold, Compat-Closure Evidence

#### Added -- Frame
- Series `loc`/`iloc` label-based and position-based selection ([`f8fa14e`](https://github.com/Dicklesworthstone/frankenpandas/commit/f8fa14ed05dbe079630040b69843e9544a9d594c))
- DataFrame `loc`/`iloc` label-based and position-based row selection ([`5ab540e`](https://github.com/Dicklesworthstone/frankenpandas/commit/5ab540e1cc4cdf4913a27aeca74a3f1ade65f0a7))

#### Added -- FrankenTUI (fp-frankentui)
- Crate scaffold with workspace integration ([`39a56dc`](https://github.com/Dicklesworthstone/frankenpandas/commit/39a56dc65f869633c4da94a27822be4d43e4a0e5))
- CLI binary entry point ([`020e198`](https://github.com/Dicklesworthstone/frankenpandas/commit/020e1986dc937adbf4aa5cf989ac7139e37aa901))
- TUI integration library (387 lines) ([`25c5dd4`](https://github.com/Dicklesworthstone/frankenpandas/commit/25c5dd48404d469e54db1299ac5e9681c8b2af23))
- E2E scenario replay and differential validation ([`104997e`](https://github.com/Dicklesworthstone/frankenpandas/commit/104997e4b74e3eee69f59509691b94c27cb9eb89))

#### Added -- Conformance
- Compat-closure evidence packs FP-P2C-006 through FP-P2C-011 ([`8788d86`](https://github.com/Dicklesworthstone/frankenpandas/commit/8788d864b13c850f63cb868b2ad74fbd2952689b))
- COMPAT-CLOSURE threat model, Rust integration plan ([`4895226`](https://github.com/Dicklesworthstone/frankenpandas/commit/4895226bd2a0eeabbc9b1a85b998b862c94c0496))

#### Changed
- Bump asupersync dependency from v0.1.1 to v0.2.0 ([`b9684f7`](https://github.com/Dicklesworthstone/frankenpandas/commit/b9684f740237a0156cd169003b5be118e7436e36))

---

### 2026-02-14 -- Core Engine Build-Out: Columnar, GroupBy, Join, Expression, Index

This date represents the most foundational build-out day, establishing the core data engine across all subsystems.

#### Added -- Frame (fp-frame)
- `Series.align()`, `combine_first()`, `reindex()` with join mode support ([`27c55cb`](https://github.com/Dicklesworthstone/frankenpandas/commit/27c55cbbc79fd088e54f1e28a773d713408f399b))
- Series arithmetic, constructors, join/concat ([`fe2fa5d`](https://github.com/Dicklesworthstone/frankenpandas/commit/fe2fa5de794b54c8752ec96a384196e21e65d7ce))

#### Added -- GroupBy (fp-groupby)
- Complete aggregation semantics: `mean`, `count`, `min`, `max`, `first`, `last` ([`79f73dc`](https://github.com/Dicklesworthstone/frankenpandas/commit/79f73dc34ee70864985b5b2e361ccf8bf4fb5fc9))
- Expanded groupby aggregation and join operations ([`60c1b7c`](https://github.com/Dicklesworthstone/frankenpandas/commit/60c1b7c9d13cf9ce75189d86a2a1cff57860ff4b))

#### Added -- Join (fp-join)
- Bumpalo arena allocator path for join intermediate vectors ([`d31a744`](https://github.com/Dicklesworthstone/frankenpandas/commit/d31a744376e0f50036021885c2b1c048292d81dc))

#### Added -- Index (fp-index)
- Complete pandas Index model with set ops, dedup, and slicing ([`513ab11`](https://github.com/Dicklesworthstone/frankenpandas/commit/513ab11d0d4928fdce84328db6d9610172b7f0a1))
- **Leapfrog triejoin** for multi-way index alignment ([`f5c2ec8`](https://github.com/Dicklesworthstone/frankenpandas/commit/f5c2ec8a0b460e848e01822522b5d08868c8c9ae))

#### Added -- Expression Engine (fp-expr)
- Incremental view maintenance with delta propagation ([`ac3208f`](https://github.com/Dicklesworthstone/frankenpandas/commit/ac3208f2d79c44bad138d0e41966a02f85f7ce06))

#### Added -- I/O (fp-io)
- **JSON I/O**, CSV options, and file-based read/write ([`9e4498f`](https://github.com/Dicklesworthstone/frankenpandas/commit/9e4498f00b3b2858713c0ab53fcb20c5b5e5c236))

#### Added -- Types (fp-types)
- Missingness utilities and nanops reductions ([`5f74f21`](https://github.com/Dicklesworthstone/frankenpandas/commit/5f74f2114ba1e7412818e9d5f511b76a9ff74fdb))

#### Added -- Columnar (fp-columnar)
- Extended columnar storage engine with new data types ([`981225d`](https://github.com/Dicklesworthstone/frankenpandas/commit/981225dd23d3ee9ca3ca12380434d6c21d9b734c))

#### Added -- Runtime (fp-runtime)
- ASUPERSYNC governance gates, property tests, and runtime skeleton ([`3013c90`](https://github.com/Dicklesworthstone/frankenpandas/commit/3013c9053c24b120001fe0707872ff57649fe37c), [`6dbfba6`](https://github.com/Dicklesworthstone/frankenpandas/commit/6dbfba61e315718bba01394aa05c1a985c818fa0))

#### Added -- Conformance (fp-conformance)
- Differential conformance harness with pandas parity testing ([`59731b8`](https://github.com/Dicklesworthstone/frankenpandas/commit/59731b8067797a58ac386f58e081f0ad00995a00))
- CI gate forensics infrastructure with machine-readable reports ([`3e8c2b7`](https://github.com/Dicklesworthstone/frankenpandas/commit/3e8c2b7bce4f50f3a89a975ae605578b3c467188))
- Conformance fixtures for FP-P2C-006 through 011 ([`dc76703`](https://github.com/Dicklesworthstone/frankenpandas/commit/dc7670312562d528034e1e482c3896b3483e5389))

---

### 2026-02-13 -- Initial Import and Performance Optimization Rounds

#### Added
- **Initial public import** of FrankenPandas clean-room Rust port ([`4e35648`](https://github.com/Dicklesworthstone/frankenpandas/commit/4e35648e4c349032ba7e2ce4604bf4cabde9f774))
- Rust-native pandas reimplementation with phase2c framework ([`b2d967a`](https://github.com/Dicklesworthstone/frankenpandas/commit/b2d967a81d8ae9e82d161a2d66ea567a22952e8b))
- Three Tier-S optimization levers (Round 3): identity-alignment fast path, dense Int64 groupby, lazy `has_duplicates` memoization ([`5ab34f8`](https://github.com/Dicklesworthstone/frankenpandas/commit/5ab34f8f9d058d75024772756e54d8cc722c7e08), [`e30f4a4`](https://github.com/Dicklesworthstone/frankenpandas/commit/e30f4a410d6e911c29a508392118a769c75eaaf9))
- Rounds 3-5 optimization evidence artifacts ([`463b234`](https://github.com/Dicklesworthstone/frankenpandas/commit/463b2347fbd99fb9a35632375ba12e8908dec426), [`6b00e16`](https://github.com/Dicklesworthstone/frankenpandas/commit/6b00e1600ffa44391ad7ba518c3fdeb1eef57f3a), [`ede5c3e`](https://github.com/Dicklesworthstone/frankenpandas/commit/ede5c3e9170ef9812aee6e2051428c8ff658eafb))
- Feature Parity Mandate for complete pandas API coverage ([`63cf641`](https://github.com/Dicklesworthstone/frankenpandas/commit/63cf641500a42a90b84541a972af5fe81c5ede24))

#### Performance
- Eliminate redundant identity casts in scalar coercion hot path ([`8a53e9a`](https://github.com/Dicklesworthstone/frankenpandas/commit/8a53e9a4d7b15c39351b5ea93960936dd12ccfad))

---

## Architecture Reference

FrankenPandas is organized as a Cargo workspace with 12 crates:

| Crate | Purpose |
|-------|---------|
| `frankenpandas` | Unified facade -- `use frankenpandas::prelude::*` |
| `fp-types` | Scalar, Column, NullKind, dtype definitions |
| `fp-columnar` | Columnar storage engine |
| `fp-frame` | DataFrame, Series, Categorical, MultiIndex integration |
| `fp-index` | Index, MultiIndex, leapfrog triejoin |
| `fp-groupby` | GroupBy, SeriesGroupBy, aggregation kernels |
| `fp-join` | Merge, concat, merge_asof, cross join |
| `fp-expr` | eval()/query() expression engine with delta propagation |
| `fp-io` | 7 I/O formats: CSV, JSON, JSONL, Parquet, Excel, SQL, Feather |
| `fp-conformance` | Differential testing harness against pandas oracle |
| `fp-runtime` | Admission policy, EvidenceLedger, governance gates |
| `fp-frankentui` | TUI integration (scaffold) |

## Commit Statistics

- **Total commits:** 269
- **Date range:** 2026-02-13 to 2026-03-17
- **Tags/releases:** None (pre-release development)
- **License:** MIT + OpenAI/Anthropic rider (adopted 2026-02-18)
