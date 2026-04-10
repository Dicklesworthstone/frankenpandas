# Changelog

All notable changes to FrankenPandas are documented in this file, organized by capability area.

FrankenPandas is a clean-room Rust reimplementation of the full pandas API surface:
12 workspace crates, 84,000+ lines of Rust, 788 public functions, zero `unsafe` code.

Repository: <https://github.com/Dicklesworthstone/frankenpandas>

**No tagged releases or GitHub releases exist.** Development spans 269 commits from
2026-02-13 to 2026-03-17 on a single `main` branch. The legacy compatibility branch diverges at
[`a9889ca`](https://github.com/Dicklesworthstone/frankenpandas/commit/a9889cafc70ec04293d907d06f0d80868263e4e8)
(MIT licensing, 2026-02-18).

---

## Table of Contents

- [Core Data Engine](#core-data-engine)
- [DataFrame API](#dataframe-api)
- [Series API](#series-api)
- [GroupBy and Aggregation](#groupby-and-aggregation)
- [Window Functions](#window-functions)
- [Join, Merge, and Concat](#join-merge-and-concat)
- [Expression Engine (eval / query)](#expression-engine-eval--query)
- [Index and MultiIndex](#index-and-multiindex)
- [I/O Formats](#io-formats)
- [Datetime and Timezone](#datetime-and-timezone)
- [Conformance Testing](#conformance-testing)
- [Runtime and Governance](#runtime-and-governance)
- [FrankenTUI (Experimental)](#frankentui-experimental)
- [Performance Optimization](#performance-optimization)
- [Licensing and Project Metadata](#licensing-and-project-metadata)

---

## Core Data Engine

The columnar storage model, type system, and null semantics that everything else builds on.

### Types and Scalars (fp-types)

- **Missingness utilities and nanops reductions** -- three-way null semantics (`Null`, `NaN`, `NaT`) matching pandas ([`5f74f21`](https://github.com/Dicklesworthstone/frankenpandas/commit/5f74f2114ba1e7412818e9d5f511b76a9ff74fdb) -- 2026-02-14)

### Columnar Storage (fp-columnar)

- **Columnar storage engine** with `Column`, `ValidityMask` (bitpacked null bitmap), and vectorized kernels ([`981225d`](https://github.com/Dicklesworthstone/frankenpandas/commit/981225dd23d3ee9ca3ca12380434d6c21d9b734c) -- 2026-02-14)
- Eliminate redundant identity casts in scalar coercion hot path ([`8a53e9a`](https://github.com/Dicklesworthstone/frankenpandas/commit/8a53e9a4d7b15c39351b5ea93960936dd12ccfad) -- 2026-02-13)

### Facade Crate (frankenpandas)

- **Top-level `frankenpandas` facade crate** with unified public API (`use frankenpandas::prelude::*`) re-exporting all subsystem types ([`74795b4`](https://github.com/Dicklesworthstone/frankenpandas/commit/74795b4cfaa30d473ed205415b67415838bad8bd) -- 2026-03-16)
- Add error types, `NullKind`, and `Column` to facade public API ([`c9d812f`](https://github.com/Dicklesworthstone/frankenpandas/commit/c9d812f31549d0593038bbbd22564541b785ea72) -- 2026-03-16)

---

## DataFrame API

The largest surface area of the project, implementing pandas DataFrame semantics in Rust.

### Constructors

- **Scalar-broadcast constructor** parity with pandas ([`c657542`](https://github.com/Dicklesworthstone/frankenpandas/commit/c657542569013492b8ce5d68de4737b4fc231ea2) -- 2026-02-19)
- **`from_records`** row-record constructor (matrix_rows) ([`ec14b61`](https://github.com/Dicklesworthstone/frankenpandas/commit/ec14b6142a5c3bf07b842305354fbe80ed6f51de) -- 2026-03-12)
- `from_tuples` and `from_tuples_with_index` ([`9ba17d7`](https://github.com/Dicklesworthstone/frankenpandas/commit/9ba17d7b9b8cc8fc5ff8ccb1e1061fa76c944be5), [`92d599c`](https://github.com/Dicklesworthstone/frankenpandas/commit/92d599c42c7e1b2c7ab8d4d6b75b41c1ff83b8eb) -- 2026-03-04)
- Duplicate index label support when all inputs are exactly aligned ([`3f028b6`](https://github.com/Dicklesworthstone/frankenpandas/commit/3f028b6f155c8f0d90518bebc2940d9280f6eea6) -- 2026-02-19)

### Selection and Indexing

- **`loc`/`iloc`** label-based and positional row selection for DataFrame ([`5ab540e`](https://github.com/Dicklesworthstone/frankenpandas/commit/5ab540e1cc4cdf4913a27aeca74a3f1ade65f0a7), [`ccd3187`](https://github.com/Dicklesworthstone/frankenpandas/commit/ccd3187887622b5a105ac9a8e00cc767538109d5) -- 2026-02-15/16)
- Boolean mask, slice, and row accessors for `loc`/`iloc`; groupby `as_index=false` ([`e28065e`](https://github.com/Dicklesworthstone/frankenpandas/commit/e28065e59ef73a17aea7f057af0cddce22095618) -- 2026-03-03)
- `loc_bool_series` and `iloc_bool_series` accessors ([`edf58dc`](https://github.com/Dicklesworthstone/frankenpandas/commit/edf58dc1f80ac3b0f15c2f3722cf8d6c19250140) -- 2026-03-03)
- Boolean mask type validation and error-path conformance support ([`12d7de2`](https://github.com/Dicklesworthstone/frankenpandas/commit/12d7de26836bf06d42f9d06e9cf76022e681b36d) -- 2026-02-16)
- `na_position` control for sort operations ([`631b3f7`](https://github.com/Dicklesworthstone/frankenpandas/commit/631b3f74e2bc430bacfbf915495aba554d1e01e2) -- 2026-03-03)

### Sorting

- **`sort_index` / `sort_values`** core APIs ([`8024fca`](https://github.com/Dicklesworthstone/frankenpandas/commit/8024fcaa556c1744d2b009eadfe1343f590ab461) -- 2026-02-18)
- Series `any`/`all` aggregation and DataFrame `sort_index`/`sort_values` parity ([`34dffda`](https://github.com/Dicklesworthstone/frankenpandas/commit/34dffda62079313f6fb00b76ca639a7aff483d75) -- 2026-02-19)

### Null Handling

- `fillna` / `dropna` null-cleaning primitives ([`555a8a9`](https://github.com/Dicklesworthstone/frankenpandas/commit/555a8a9d15848cc14f34263ae3b3cb0d21f1d71a) -- 2026-02-19)
- `isna` / `notna` null-mask APIs ([`9d27dda`](https://github.com/Dicklesworthstone/frankenpandas/commit/9d27ddabe203fd8cf6da18b9616d418f68f354f5) -- 2026-02-19)
- pandas `isnull`/`notnull` aliases ([`364077f`](https://github.com/Dicklesworthstone/frankenpandas/commit/364077f27091357c16a700212a4b418626f26d97) -- 2026-02-19)
- `dropna` with `how`/`subset` options ([`9e9090b`](https://github.com/Dicklesworthstone/frankenpandas/commit/9e9090b8cd0a4edfde8c3d458d9c04d328c354a9) -- 2026-02-19)
- `dropna` axis=1 options and defaults ([`1e020cd`](https://github.com/Dicklesworthstone/frankenpandas/commit/1e020cd8ec9d0f47589a1bf234db0654fada96ff), [`364077f`](https://github.com/Dicklesworthstone/frankenpandas/commit/364077f27091357c16a700212a4b418626f26d97) -- 2026-02-19)
- `dropna` thresh semantics for rows and columns ([`8a3a971`](https://github.com/Dicklesworthstone/frankenpandas/commit/8a3a97151b3c3d4ff75f3e59df3071d30240a081) -- 2026-02-19)
- `fillna_dict` and `fillna_limit` ([`ed701c0`](https://github.com/Dicklesworthstone/frankenpandas/commit/ed701c0e71a47dce63685a22b6ec0aae01a2fe77), [`1b0d699`](https://github.com/Dicklesworthstone/frankenpandas/commit/1b0d699310dbcda42d346f3985ac5e8b1e29bdca) -- 2026-03-03/04)
- `count_na` ([`aa6e20b`](https://github.com/Dicklesworthstone/frankenpandas/commit/aa6e20bd2f7723992f558a731af1c010a1559cd5) -- 2026-03-04)

### Type Coercion

- **`astype`** and **`astype_safe`** coercion APIs for Series and DataFrame ([`31fa57e`](https://github.com/Dicklesworthstone/frankenpandas/commit/31fa57efb11a2316e45a19ac8b8bad3a6e01e38f) -- 2026-02-19)
- Multi-column `astype` mapping parity ([`8d53b84`](https://github.com/Dicklesworthstone/frankenpandas/commit/8d53b8425a2f5023c016f3b41d133e1f10c33135) -- 2026-02-19)
- `astype`, `astype_safe`, `rolling_with_center` ([`ad90b46`](https://github.com/Dicklesworthstone/frankenpandas/commit/ad90b46f12326635a4c66fa2ed49619c3719a2f4) -- 2026-03-04)
- `convert_dtypes` ([`0b7bc94`](https://github.com/Dicklesworthstone/frankenpandas/commit/0b7bc94d8ffd2ad39e00a36d362e82b179bc74b8) -- 2026-02-26)

### Reshaping and Pivoting

- **`rank`**, **`melt`**, **`pivot_table`** ([`158b546`](https://github.com/Dicklesworthstone/frankenpandas/commit/158b54669dff4a1ee7e9335d9f30a7fa89f229e5) -- 2026-02-25)
- **`stack` / `unstack`**, closure-based `apply_fn`, `map_values` ([`49c7417`](https://github.com/Dicklesworthstone/frankenpandas/commit/49c7417db3f25b6aab6c609f563b9091abd663fd) -- 2026-02-25)
- `pivot_table_with_margins`, `interpolate_method` ([`ed701c0`](https://github.com/Dicklesworthstone/frankenpandas/commit/ed701c0e71a47dce63685a22b6ec0aae01a2fe77) -- 2026-03-03)
- `pivot_table_multi_agg` ([`64eacff`](https://github.com/Dicklesworthstone/frankenpandas/commit/64eacffd6524f4ce4bba4de4b9d92d3fc185624c) -- 2026-03-03)
- `pivot_table_fill`, `swaplevel` ([`aa6e20b`](https://github.com/Dicklesworthstone/frankenpandas/commit/aa6e20bd2f7723992f558a731af1c010a1559cd5) -- 2026-03-04)
- `pivot_table_multi_values` ([`1fcb08f`](https://github.com/Dicklesworthstone/frankenpandas/commit/1fcb08f1f7fa8af752bc8524295adf721dccf431) -- 2026-03-04)
- `get_dummies` ([`15393ef`](https://github.com/Dicklesworthstone/frankenpandas/commit/15393ef2fa121f2afdb474ecf1c55d7de1bd96a4) -- 2026-02-26)

### Statistics and Aggregation

- **`agg`**, `applymap`, `transform`, `corr`, `cov`, `nlargest`, `nsmallest`, `reindex` ([`4dc605f`](https://github.com/Dicklesworthstone/frankenpandas/commit/4dc605fa069ef7f66cb7c5a219242657aad2a223) -- 2026-02-25)
- `std_agg_ddof`, `var_agg_ddof` for configurable degrees of freedom ([`f584f56`](https://github.com/Dicklesworthstone/frankenpandas/commit/f584f5690fcf83988f0784ae387a05744eb806d0) -- 2026-03-03)
- `corr_min_periods`, `cov_min_periods` ([`bdc49e8`](https://github.com/Dicklesworthstone/frankenpandas/commit/bdc49e8de25e6974c63eb3514a6bb833c53e710a) -- 2026-03-03)
- `corrwith_axis`, `map_with_default` ([`5a2bc08`](https://github.com/Dicklesworthstone/frankenpandas/commit/5a2bc08640ecf20e8a0fd3263386f25a2c7b099f) -- 2026-03-03)
- `skipna` aggregates, `first`/`last` offset, date arithmetic helpers ([`8cebdeb`](https://github.com/Dicklesworthstone/frankenpandas/commit/8cebdeb64b5ef0865adae1b3af28465617a3f458) -- 2026-03-04)
- `describe` with dtype filtering and `describe_dtypes` ([`d520b8f`](https://github.com/Dicklesworthstone/frankenpandas/commit/d520b8faca28b28202fcdfc6b79a713fc46e0008), [`ccd59f4`](https://github.com/Dicklesworthstone/frankenpandas/commit/ccd59f4e90265e130097f69f7d5fe67813a7989e) -- 2026-03-03/04)
- `info` (memory/shape introspection) ([`0034f3d`](https://github.com/Dicklesworthstone/frankenpandas/commit/0034f3d1a586b581c7a28e441f6a094d2419baee) -- 2026-02-25)
- `sample` (random row sampling) ([`0034f3d`](https://github.com/Dicklesworthstone/frankenpandas/commit/0034f3d1a586b581c7a28e441f6a094d2419baee) -- 2026-02-25)
- `sample_weights` ([`82229b0`](https://github.com/Dicklesworthstone/frankenpandas/commit/82229b0ebf2a7d6a76bb1768e70e732fc2771dd4) -- 2026-03-04)

### Mutation and Assignment

- **`where`**, **`mask`**, `iterrows`, `itertuples`, `items`, `assign`, `pipe` ([`c23f3b3`](https://github.com/Dicklesworthstone/frankenpandas/commit/c23f3b366df374e8f4eb17e405f3f075e858cc9c) -- 2026-02-25)
- `where_cond_series`, `mask_series` ([`1b0d699`](https://github.com/Dicklesworthstone/frankenpandas/commit/1b0d699310dbcda42d346f3985ac5e8b1e29bdca) -- 2026-03-04)
- `assign_fn`, `applymap_na_action`, `isin_dict` ([`c782fc7`](https://github.com/Dicklesworthstone/frankenpandas/commit/c782fc78b33a2e7f952e21b8efcc65ae94b710f3) -- 2026-03-03)
- `replace_dict`, `replace_regex` ([`631b3f7`](https://github.com/Dicklesworthstone/frankenpandas/commit/631b3f74e2bc430bacfbf915495aba554d1e01e2), [`bf805b0`](https://github.com/Dicklesworthstone/frankenpandas/commit/bf805b0ae8983056899575e759cc430bc8ab609c) -- 2026-03-03/04)
- `replace_map`, `apply_fn_na` ([`70c2196`](https://github.com/Dicklesworthstone/frankenpandas/commit/70c21965c844adbcdd250cefa04f39651862b473) -- 2026-03-04)

### Arithmetic and Element-Wise Operations

- **`combine`** for element-wise binary operations ([`34e8a35`](https://github.com/Dicklesworthstone/frankenpandas/commit/34e8a359ce79a90441134ee4568c0481ebfeda18) -- 2026-03-03)
- `pow_df` ([`7337bba`](https://github.com/Dicklesworthstone/frankenpandas/commit/7337bbae7c1a13418f3e2f0007bd20ec4155a21a) -- 2026-03-03)
- Fill arithmetic (`radd`/`rsub`/`rmul`/`rdiv`), `extract_to_frame`, `margins_name` ([`a69d85e`](https://github.com/Dicklesworthstone/frankenpandas/commit/a69d85efebac24dda8724214535d23759162dd64) -- 2026-03-03)
- Comparison ops ([`15393ef`](https://github.com/Dicklesworthstone/frankenpandas/commit/15393ef2fa121f2afdb474ecf1c55d7de1bd96a4) -- 2026-02-26)

### Duplicate Handling

- `duplicated` / `drop_duplicates` DataFrame parity ([`738d61b`](https://github.com/Dicklesworthstone/frankenpandas/commit/738d61b7d0b9ce22f059c6b05a2e113691209f52) -- 2026-02-19)

### Index Manipulation

- `set_index` / `reset_index` parity APIs ([`d005c31`](https://github.com/Dicklesworthstone/frankenpandas/commit/d005c311e164cac7409df9faa954f369be19672b) -- 2026-02-19)
- `rename_index` ([`ef41f54`](https://github.com/Dicklesworthstone/frankenpandas/commit/ef41f54cdfbc64a26d055e5d046a728ead60f690) -- 2026-03-03)
- Mixed Int64/Utf8 index labels in `reset_index` ([`82e9fbb`](https://github.com/Dicklesworthstone/frankenpandas/commit/82e9fbb7d3a283e3d06ddb0eb1b63d07a224dcfb) -- 2026-02-19)

### Reindexing and Alignment

- **`Series.align()`**, `combine_first()`, `reindex()` with join mode support ([`27c55cb`](https://github.com/Dicklesworthstone/frankenpandas/commit/27c55cbbc79fd088e54f1e28a773d713408f399b) -- 2026-02-14)
- `reindex_with_method` and groupby `sort` parameter ([`ba24056`](https://github.com/Dicklesworthstone/frankenpandas/commit/ba24056fb4bee776a442913bacaa6a4153e6f1d8) -- 2026-03-03)
- `reindex_columns` ([`9ba17d7`](https://github.com/Dicklesworthstone/frankenpandas/commit/9ba17d7b9b8cc8fc5ff8ccb1e1061fa76c944be5) -- 2026-03-04)
- `reindex_fill`, rolling center mode ([`1fcb08f`](https://github.com/Dicklesworthstone/frankenpandas/commit/1fcb08f1f7fa8af752bc8524295adf721dccf431) -- 2026-03-04)

### Display and Export

- `truncate`, `to_string_truncated` ([`bf805b0`](https://github.com/Dicklesworthstone/frankenpandas/commit/bf805b0ae8983056899575e759cc430bc8ab609c) -- 2026-03-04)
- `to_csv_options` ([`d520b8f`](https://github.com/Dicklesworthstone/frankenpandas/commit/d520b8faca28b28202fcdfc6b79a713fc46e0008) -- 2026-03-03)
- `to_dict` tight orient, `to_series_dict`, label display fixes ([`4872463`](https://github.com/Dicklesworthstone/frankenpandas/commit/487246313c0b3dab55f35a30d06f88c96a63aa97) -- 2026-03-15)
- `to_json` orient parity with pandas ([`e1352d9`](https://github.com/Dicklesworthstone/frankenpandas/commit/e1352d9fcc6fe97b9bc743bc54f10e58f2606995) -- 2026-03-12)
- `to_records`, `to_numpy_2d` ([`753ea6c`](https://github.com/Dicklesworthstone/frankenpandas/commit/753ea6c21d8e18ad5dbb3c6ec709152e3ed25d38), [`bc27b5f`](https://github.com/Dicklesworthstone/frankenpandas/commit/bc27b5f53b6bdea260ab814867a50c3bdf9c7f68) -- 2026-03-03/04)

### Misc DataFrame Methods

- `drop_columns`, `clip_with_series`, `nlargest_multi` ([`86b31a2`](https://github.com/Dicklesworthstone/frankenpandas/commit/86b31a24e5a50fca5914e41b2e911e3bf1b465f8) -- 2026-03-04)
- `ignore_index` concat ([`631b3f7`](https://github.com/Dicklesworthstone/frankenpandas/commit/631b3f74e2bc430bacfbf915495aba554d1e01e2) -- 2026-03-03)
- `value_counts_subset` ([`ccd59f4`](https://github.com/Dicklesworthstone/frankenpandas/commit/ccd59f4e90265e130097f69f7d5fe67813a7989e) -- 2026-03-04)
- Row-wise `var`/`std`/`median` in `DataFrame.apply`, typed Scalar returns ([`2f7f5a4`](https://github.com/Dicklesworthstone/frankenpandas/commit/2f7f5a45153e68c7abdd1dce3069ecc17cde1591), [`ee48142`](https://github.com/Dicklesworthstone/frankenpandas/commit/ee481424ea601b1c22bcfa2da0f2ed71d650af87) -- 2026-02-25)
- Major frame implementation rewrite and expression engine integration ([`ae437eb`](https://github.com/Dicklesworthstone/frankenpandas/commit/ae437eb70b7d0feef793427a17f25179ccb55637) -- 2026-02-25)

### DataFrame Bug Fixes

- `describe()` type mismatch for string column statistics ([`dd638ad`](https://github.com/Dicklesworthstone/frankenpandas/commit/dd638ade448eaa394186b680d9d610bb6d83045a) -- 2026-03-04)
- Filter non-numeric columns in `corr`/`cov` pairwise_stat ([`26e5e7e`](https://github.com/Dicklesworthstone/frankenpandas/commit/26e5e7e9e28bba1df5a69b1684f41c68cd4aa3f0) -- 2026-02-25)
- DataFrame arithmetic ops, type coercion, label width, diff comparison ([`3ba9008`](https://github.com/Dicklesworthstone/frankenpandas/commit/3ba900865bf8aa28222b4853c913acfe0977c5f9) -- 2026-02-26)
- `Column::from_values` in applymap, groupby count test ([`c31c3d8`](https://github.com/Dicklesworthstone/frankenpandas/commit/c31c3d881944d89a201026fc0bf7f825ba699e08) -- 2026-02-25)
- Dead branches in `pct_change` and `rolling_count` ([`531cfb6`](https://github.com/Dicklesworthstone/frankenpandas/commit/531cfb655403aaed746636940f328f7799818cb9) -- 2026-02-25)
- Bool variant in debug column name parser ([`607ab58`](https://github.com/Dicklesworthstone/frankenpandas/commit/607ab582ce3501d6e74e7f0a2a454a233c443e16) -- 2026-02-25)
- Clippy warnings in `interpolate_method` and `pivot_table_with_margins` ([`a030483`](https://github.com/Dicklesworthstone/frankenpandas/commit/a030483458fa3eee3bb405d1d25ac26f35079bc4) -- 2026-03-03)
- Explicit Scalar constructors in isin_dict and applymap tests ([`f7a0784`](https://github.com/Dicklesworthstone/frankenpandas/commit/f7a078404245292cc41bd06c91a10d276e63082c) -- 2026-03-03)
- Explicit dereference in `max_by_key` closure ([`3b22069`](https://github.com/Dicklesworthstone/frankenpandas/commit/3b2206959945b70be4adfbd51fe632a190996a5a) -- 2026-03-04)
- Preserve column order and dtype through subsetting, fix outer join performance ([`d3b1542`](https://github.com/Dicklesworthstone/frankenpandas/commit/d3b1542517942673685229b689f059d024be673c) -- 2026-02-20)
- Format bool groupby key labels as pandas-style `"True"` / `"False"` ([`44dab10`](https://github.com/Dicklesworthstone/frankenpandas/commit/44dab10a14b084890d63545222db789f447ab89d) -- 2026-03-02)
- `to_multi_index` now handles Float64/Bool columns correctly ([`dc8cf48`](https://github.com/Dicklesworthstone/frankenpandas/commit/dc8cf4805dbbe9fbde34a73db5a2dfc4c97184e2) -- 2026-03-15)

---

## Series API

### Core Series Operations

- Series arithmetic, constructors, join/concat foundation ([`fe2fa5d`](https://github.com/Dicklesworthstone/frankenpandas/commit/fe2fa5de794b54c8752ec96a384196e21e65d7ce) -- 2026-02-14)
- 15 new Series methods and 4 new DataFrame methods, groupby/types conformance fixes ([`22eb92a`](https://github.com/Dicklesworthstone/frankenpandas/commit/22eb92a3b8566afe619cbd614378d00967d95a0e) -- 2026-02-21)
- `head`/`tail` API parity ([`dae33f3`](https://github.com/Dicklesworthstone/frankenpandas/commit/dae33f3c634b984e87cccf9b4e14db9e4366f732) -- 2026-02-19)
- `value_counts`, `sort`, `tail`, `isna`/`notna`, `dropna`/`fillna`, `count`, `isnull`/`notnull` ([`f1316f2`](https://github.com/Dicklesworthstone/frankenpandas/commit/f1316f2a8f974057adc3defcc199259eb5f0fbc9) -- 2026-02-20)
- `any`/`all` aggregation ([`34dffda`](https://github.com/Dicklesworthstone/frankenpandas/commit/34dffda62079313f6fb00b76ca639a7aff483d75) -- 2026-02-19)
- `item()` (extract single element) ([`ef41f54`](https://github.com/Dicklesworthstone/frankenpandas/commit/ef41f54cdfbc64a26d055e5d046a728ead60f690) -- 2026-03-03)

### Selection and Filtering

- **Series `loc`/`iloc`** label-based and position-based selection ([`f8fa14e`](https://github.com/Dicklesworthstone/frankenpandas/commit/f8fa14ed05dbe079630040b69843e9544a9d594c) -- 2026-02-15)
- `where_cond`, `mask`, `isin`, `between` ([`ddf8726`](https://github.com/Dicklesworthstone/frankenpandas/commit/ddf87260109b28e99cba825096c512724f7bd2a6) -- 2026-02-25)
- `case_when`, `map_with_na_action` ([`0b7bc94`](https://github.com/Dicklesworthstone/frankenpandas/commit/0b7bc94d8ffd2ad39e00a36d362e82b179bc74b8) -- 2026-02-26)

### Statistics

- `idxmin`, `idxmax`, `nlargest`, `nsmallest`, `pct_change` ([`edba19e`](https://github.com/Dicklesworthstone/frankenpandas/commit/edba19ec4da03004cde3b6a0d93b098abeaa654d) -- 2026-02-25)
- `var_ddof`, `std_ddof` for configurable degrees of freedom ([`d5b6aa6`](https://github.com/Dicklesworthstone/frankenpandas/commit/d5b6aa6cd242330133c982ebb5582546b0e3972f) -- 2026-03-03)
- `describe` and `drop_duplicates_keep` ([`d520b8f`](https://github.com/Dicklesworthstone/frankenpandas/commit/d520b8faca28b28202fcdfc6b79a713fc46e0008) -- 2026-03-03)
- `value_counts_bins` for binned frequency counting ([`93d042e`](https://github.com/Dicklesworthstone/frankenpandas/commit/93d042e78c174ea79ae59f8568b8fcfc42ec091e) -- 2026-03-03)

### Duplicate Handling

- `duplicated_keep` with `DuplicateKeep` parameter ([`809c243`](https://github.com/Dicklesworthstone/frankenpandas/commit/809c243de94f73c2aa45c5d228e3949061148e0d) -- 2026-03-03)

### String Operations

- **Series `str` accessor** with 15 string operations ([`e2224bf`](https://github.com/Dicklesworthstone/frankenpandas/commit/e2224bf386d75e0b29eda19ecb7ec70edc9f4bc8) -- 2026-02-25)
- String accessor extensions ([`15393ef`](https://github.com/Dicklesworthstone/frankenpandas/commit/15393ef2fa121f2afdb474ecf1c55d7de1bd96a4) -- 2026-02-26)
- `replace_regex`, `slice_replace` ([`fc0f0a6`](https://github.com/Dicklesworthstone/frankenpandas/commit/fc0f0a6c7e8944883411d9a85e5e185a1a0d73af), [`ccd59f4`](https://github.com/Dicklesworthstone/frankenpandas/commit/ccd59f4e90265e130097f69f7d5fe67813a7989e) -- 2026-03-03/04)

### Series Aggregation

- **`agg()`** multi-function aggregation and **`groupby()`** ([`6b7a610`](https://github.com/Dicklesworthstone/frankenpandas/commit/6b7a61003c5b632fdca22fe5f8550a4aa7ceeaca), [`7337bba`](https://github.com/Dicklesworthstone/frankenpandas/commit/7337bbae7c1a13418f3e2f0007bd20ec4155a21a) -- 2026-03-03)

---

## GroupBy and Aggregation

### Core Aggregation (fp-groupby)

- **Complete aggregation semantics:** `mean`, `count`, `min`, `max`, `first`, `last` ([`79f73dc`](https://github.com/Dicklesworthstone/frankenpandas/commit/79f73dc34ee70864985b5b2e361ccf8bf4fb5fc9) -- 2026-02-14)
- Expanded groupby aggregation and join operations ([`60c1b7c`](https://github.com/Dicklesworthstone/frankenpandas/commit/60c1b7c9d13cf9ce75189d86a2a1cff57860ff4b) -- 2026-02-14)
- `nunique`, `prod`, `size` aggregation functions ([`371d638`](https://github.com/Dicklesworthstone/frankenpandas/commit/371d638e6042e4e36ff6231aca8036c6289eb073) -- 2026-02-25)
- `std`/`var`/`median` operations ([`c6fef8d`](https://github.com/Dicklesworthstone/frankenpandas/commit/c6fef8d50e75f0ce5b67e32fe1ce86488d0423d8) -- 2026-02-17)
- Negative `iloc` positions in groupby context ([`c6fef8d`](https://github.com/Dicklesworthstone/frankenpandas/commit/c6fef8d50e75f0ce5b67e32fe1ce86488d0423d8) -- 2026-02-17)

### DataFrame GroupBy Integration

- DataFrame groupby integration ([`0034f3d`](https://github.com/Dicklesworthstone/frankenpandas/commit/0034f3d1a586b581c7a28e441f6a094d2419baee) -- 2026-02-25)
- GroupBy `sort` parameter ([`ba24056`](https://github.com/Dicklesworthstone/frankenpandas/commit/ba24056fb4bee776a442913bacaa6a4153e6f1d8) -- 2026-03-03)
- GroupBy `dropna` parameter ([`82229b0`](https://github.com/Dicklesworthstone/frankenpandas/commit/82229b0ebf2a7d6a76bb1768e70e732fc2771dd4) -- 2026-03-04)
- GroupBy `as_index=false` support ([`e28065e`](https://github.com/Dicklesworthstone/frankenpandas/commit/e28065e59ef73a17aea7f057af0cddce22095618) -- 2026-03-03)
- GroupBy `agg_multi` ([`631b3f7`](https://github.com/Dicklesworthstone/frankenpandas/commit/631b3f74e2bc430bacfbf915495aba554d1e01e2) -- 2026-03-03)
- Runtime admission policy observability wired into groupby ([`ff6128d`](https://github.com/Dicklesworthstone/frankenpandas/commit/ff6128d3a488ad1c2d49d56fb90c027d51adbed0) -- 2026-03-02)

### Named Aggregation

- **`DataFrameGroupBy.agg_named()`** for pandas-style named aggregation with `(output_name, column, func)` tuples ([`ef15a82`](https://github.com/Dicklesworthstone/frankenpandas/commit/ef15a825df581b9150fe50bd320cfdae5465ff30) -- 2026-03-17)
- `agg_named` rejects duplicate output column names ([`fbf21b8`](https://github.com/Dicklesworthstone/frankenpandas/commit/fbf21b8fcb4ec39dc5bdd73ac1dd357f315dd946) -- 2026-03-17)

### SeriesGroupBy

- `SeriesGroupBy.agg`, `var`, `median`, `prod` ([`fc0f0a6`](https://github.com/Dicklesworthstone/frankenpandas/commit/fc0f0a6c7e8944883411d9a85e5e185a1a0d73af), [`9ba17d7`](https://github.com/Dicklesworthstone/frankenpandas/commit/9ba17d7b9b8cc8fc5ff8ccb1e1061fa76c944be5) -- 2026-03-04)
- Resample multi-aggregate ([`70c2196`](https://github.com/Dicklesworthstone/frankenpandas/commit/70c21965c844adbcdd250cefa04f39651862b473) -- 2026-03-04)
- GroupBy rolling/resample ([`59374a2`](https://github.com/Dicklesworthstone/frankenpandas/commit/59374a2ee11587f1a48651de8e69fdb43cf7d50b) -- 2026-03-04)

### Massive API Expansion

- Massive Series/DataFrame/GroupBy API expansion, `merge_asof`, and EWM windows ([`c41d37d`](https://github.com/Dicklesworthstone/frankenpandas/commit/c41d37d0bc6f2d739eaa806538fe73038a838521) -- 2026-02-26)
- Expand Series/DataFrame API surface with `dt` accessor, element-wise ops, merge trait, and string methods ([`d3fb430`](https://github.com/Dicklesworthstone/frankenpandas/commit/d3fb430a981d9bf0a3d8a84942dfa047f0a6247c) -- 2026-02-26)

---

## Window Functions

### Rolling Windows

- **Rolling/expanding window operations** for Series ([`e82e531`](https://github.com/Dicklesworthstone/frankenpandas/commit/e82e5315d9f4a7667472d9340fdaa99889e0f6c7) -- 2026-02-25)
- Rolling `skew`/`kurt` ([`ef41f54`](https://github.com/Dicklesworthstone/frankenpandas/commit/ef41f54cdfbc64a26d055e5d046a728ead60f690) -- 2026-03-03)
- Rolling `corr`/`cov` ([`15393ef`](https://github.com/Dicklesworthstone/frankenpandas/commit/15393ef2fa121f2afdb474ecf1c55d7de1bd96a4) -- 2026-02-26)
- `rolling.agg` ([`bc27b5f`](https://github.com/Dicklesworthstone/frankenpandas/commit/bc27b5f53b6bdea260ab814867a50c3bdf9c7f68) -- 2026-03-04)
- `rolling_with_center` ([`ad90b46`](https://github.com/Dicklesworthstone/frankenpandas/commit/ad90b46f12326635a4c66fa2ed49619c3719a2f4) -- 2026-03-04)

### Expanding Windows

- Expanding `skew`/`kurt` ([`f7d419c`](https://github.com/Dicklesworthstone/frankenpandas/commit/f7d419caf5eae30214b2d6c15df406371cba586f), [`753ea6c`](https://github.com/Dicklesworthstone/frankenpandas/commit/753ea6c21d8e18ad5dbb3c6ec709152e3ed25d38) -- 2026-03-03)

### Exponentially Weighted Windows

- **EWM (exponentially weighted) windows** ([`c41d37d`](https://github.com/Dicklesworthstone/frankenpandas/commit/c41d37d0bc6f2d739eaa806538fe73038a838521) -- 2026-02-26)

---

## Join, Merge, and Concat

### Core Join Engine (fp-join)

- Bumpalo arena allocator path for join intermediate vectors ([`d31a744`](https://github.com/Dicklesworthstone/frankenpandas/commit/d31a744376e0f50036021885c2b1c048292d81dc) -- 2026-02-14)
- **Cross join** semantics ([`8037afa`](https://github.com/Dicklesworthstone/frankenpandas/commit/8037afa227a42d5be84b94e244293cca2c803a1a) -- 2026-02-18)
- **`merge_asof`** ([`c41d37d`](https://github.com/Dicklesworthstone/frankenpandas/commit/c41d37d0bc6f2d739eaa806538fe73038a838521) -- 2026-02-26)

### Merge Parity Options

- Merge `indicator` parity (pandas `_merge` column) ([`d71abd5`](https://github.com/Dicklesworthstone/frankenpandas/commit/d71abd5f1e69afb195020779e69a31a31db91475) -- 2026-02-18)
- Merge `validate` parity ([`c0007a0`](https://github.com/Dicklesworthstone/frankenpandas/commit/c0007a088003724939c234119f98e43134b6e55a) -- 2026-02-18)
- Merge `suffixes` tuple parity and collision semantics ([`56ffb80`](https://github.com/Dicklesworthstone/frankenpandas/commit/56ffb80ba5e1035ac2dd329279ce43278372b3e9) -- 2026-02-18)
- Merge `sort` flag parity ([`14da9af`](https://github.com/Dicklesworthstone/frankenpandas/commit/14da9af669c1d05e6f5943b085e8e333a1e89e40) -- 2026-02-18)
- Reject series_join cross, negative packet coverage ([`597cffc`](https://github.com/Dicklesworthstone/frankenpandas/commit/597cffc19b9c9fc06856e4d5a8b46ee6ce665b0b) -- 2026-02-18)

### Join Bug Fixes

- Preserve float precision in join keys ([`54cef39`](https://github.com/Dicklesworthstone/frankenpandas/commit/54cef3982d84068f232ac02638ed0531d0c5cbcc) -- 2026-02-20)
- Fix outer join performance ([`d3b1542`](https://github.com/Dicklesworthstone/frankenpandas/commit/d3b1542517942673685229b689f059d024be673c) -- 2026-02-20)

---

## Expression Engine (eval / query)

The `fp-expr` crate implements pandas-compatible `eval()` and `query()` string expression parsing and evaluation.

### Operators and Evaluation

- **Incremental view maintenance** with delta propagation ([`ac3208f`](https://github.com/Dicklesworthstone/frankenpandas/commit/ac3208f2d79c44bad138d0e41966a02f85f7ce06) -- 2026-02-14)
- `Sub`/`Mul`/`Div` expression operators ([`f389846`](https://github.com/Dicklesworthstone/frankenpandas/commit/f3898463ca834baa0e5f77b6e673c19d998d1fda) -- 2026-02-18)
- Comparison expression operators and delta evaluation ([`626267d`](https://github.com/Dicklesworthstone/frankenpandas/commit/626267d267077368c190dc6e4bdb16a946823404) -- 2026-02-19)
- Boolean mask composition operators and planner logical ops ([`03fc784`](https://github.com/Dicklesworthstone/frankenpandas/commit/03fc784b5f4c40821dea314f1dbd7473c74d30c3) -- 2026-02-19)

### DataFrame Integration

- DataFrame-backed evaluation bridge via `EvalContext` ([`85ecf53`](https://github.com/Dicklesworthstone/frankenpandas/commit/85ecf53b30e16355596caae67ef938641f3b3d00) -- 2026-02-19)
- DataFrame `query()`-style filter helper ([`58cd95a`](https://github.com/Dicklesworthstone/frankenpandas/commit/58cd95afaa732b8afd47136cb37ee5928efb4f5a) -- 2026-02-19)
- **`@local` variable bindings** in expression engine for `eval()` / `query()` ([`49a14c2`](https://github.com/Dicklesworthstone/frankenpandas/commit/49a14c244ed00ffd23ec8c70078111f3c7f2423d) -- 2026-03-10)

### Expression Bug Fixes

- Rewrite expression linearity detection, refactor groupby to preserve Scalar types ([`17b6fef`](https://github.com/Dicklesworthstone/frankenpandas/commit/17b6fef7a90bde469f303163701664ea9f6cd5f2) -- 2026-02-20)

---

## Index and MultiIndex

### Index Foundation (fp-index)

- **Complete pandas Index model** with set ops, dedup, and slicing ([`513ab11`](https://github.com/Dicklesworthstone/frankenpandas/commit/513ab11d0d4928fdce84328db6d9610172b7f0a1) -- 2026-02-14)
- **Leapfrog triejoin** for multi-way index alignment ([`f5c2ec8`](https://github.com/Dicklesworthstone/frankenpandas/commit/f5c2ec8a0b460e848e01822522b5d08868c8c9ae) -- 2026-02-14)
- `Index.name`, `map_series`, `names`, `to_flat_index` ([`59374a2`](https://github.com/Dicklesworthstone/frankenpandas/commit/59374a2ee11587f1a48651de8e69fdb43cf7d50b), [`ad90b46`](https://github.com/Dicklesworthstone/frankenpandas/commit/ad90b46f12326635a4c66fa2ed49619c3719a2f4) -- 2026-03-04)
- Index utility methods ([`0b7bc94`](https://github.com/Dicklesworthstone/frankenpandas/commit/0b7bc94d8ffd2ad39e00a36d362e82b179bc74b8) -- 2026-02-26)

### MultiIndex

- **MultiIndex foundation** for hierarchical indexing ([`80e8d00`](https://github.com/Dicklesworthstone/frankenpandas/commit/80e8d00eac216dd7874b579d6cad44f6dcebefcf) -- 2026-03-15)
- MultiIndex integrated with DataFrame `set_index` / `reset_index` ([`17777c9`](https://github.com/Dicklesworthstone/frankenpandas/commit/17777c939d89ae7e6513773681bf8604c660effd) -- 2026-03-15)
- **`MultiIndex.reorder_levels()`** ([`11eb0b7`](https://github.com/Dicklesworthstone/frankenpandas/commit/11eb0b7a7f0d3630e5872d8d92e0e0e135dc4bd0) -- 2026-03-16)

### Categorical Dtype

- **Categorical dtype** support as Series metadata layer ([`7a384c5`](https://github.com/Dicklesworthstone/frankenpandas/commit/7a384c5c254bda2646615ba0b16374abfe7e1135) -- 2026-03-15)

---

## I/O Formats

Seven I/O formats, all accessible through the `DataFrameIoExt` trait.

### CSV

- **CSV I/O** with options and file-based read/write ([`9e4498f`](https://github.com/Dicklesworthstone/frankenpandas/commit/9e4498f00b3b2858713c0ab53fcb20c5b5e5c236) -- 2026-02-14)
- File-path CSV options reader ([`d61d610`](https://github.com/Dicklesworthstone/frankenpandas/commit/d61d61064b63c5c5cc524ae2e1c3f36b79d57363) -- 2026-02-19)
- Headerless CSV input with auto-generated column names ([`2694498`](https://github.com/Dicklesworthstone/frankenpandas/commit/2694498db8bbdbc495df320bda93a7b9489a3a9d) -- 2026-03-03)
- `CsvReadOptions` expanded with `usecols`, `nrows`, `skiprows`, `dtype` (pandas `read_csv` parity) ([`8e21aa5`](https://github.com/Dicklesworthstone/frankenpandas/commit/8e21aa54b44eb912a33298e4960d42c8bdbcce2f), [`f648113`](https://github.com/Dicklesworthstone/frankenpandas/commit/f6481139e08d2b0fb537ded6afab74ba0d83eb93) -- 2026-03-16)
- `MissingIndexColumn` error variant and CSV edge-case tests ([`d3b09c0`](https://github.com/Dicklesworthstone/frankenpandas/commit/d3b09c0358969a900478f039dbd944305b1c55d9) -- 2026-02-16)

### JSON

- **JSON I/O** with 5 orients (Records, Columns, Index, Split, Values) ([`9e4498f`](https://github.com/Dicklesworthstone/frankenpandas/commit/9e4498f00b3b2858713c0ab53fcb20c5b5e5c236) -- 2026-02-14)
- JSON orient=index read/write path ([`3cda217`](https://github.com/Dicklesworthstone/frankenpandas/commit/3cda217e440d79d58140e4a5590e9cb2f84644c6) -- 2026-02-19)
- Preserve split-orient JSON index labels on roundtrip ([`e324230`](https://github.com/Dicklesworthstone/frankenpandas/commit/e324230d40ce5f628038cfad4d1f26a89d247b4e) -- 2026-02-19)
- Align JSON orients with pandas semantics ([`9f46f2d`](https://github.com/Dicklesworthstone/frankenpandas/commit/9f46f2da5cca00ba3d518f31bb1dbb81416c3bff) -- 2026-03-12)

### JSONL (JSON Lines)

- **JSONL read/write I/O** (7th format), one object per line, blank-line tolerant ([`30156ec`](https://github.com/Dicklesworthstone/frankenpandas/commit/30156ec2f0f24cb52f44778ce948fb3c9f6918d4) -- 2026-03-16)
- JSONL reader now unions all keys across rows for ragged-schema support ([`d5b4e24`](https://github.com/Dicklesworthstone/frankenpandas/commit/d5b4e2447ce2e51918493167878211888d9d3700) -- 2026-03-16)

### Parquet

- **Parquet read/write** via Arrow RecordBatch integration ([`262235b`](https://github.com/Dicklesworthstone/frankenpandas/commit/262235b14594bf21e586841ad423507439da6d69) -- 2026-02-25)

### Excel

- **Excel I/O** (.xlsx/.xls/.xlsb/.ods) with property-based fuzz tests ([`49aa1b6`](https://github.com/Dicklesworthstone/frankenpandas/commit/49aa1b63d3ca7c6e22d06be474fd12a4c27358bf) -- 2026-03-15)

### SQL (SQLite)

- **SQL I/O** via rusqlite -- `read_sql`/`to_sql` with `SqlIfExists` (Fail/Replace/Append) ([`df38cff`](https://github.com/Dicklesworthstone/frankenpandas/commit/df38cffa052c038ae5f660fbe0ed1dfd122f8df2) -- 2026-03-15)
- Reject empty SQL table names in validation ([`82e24e9`](https://github.com/Dicklesworthstone/frankenpandas/commit/82e24e91c5d7d8bd5e3fb6e15d26ed7c9ce5202d) -- 2026-03-15)

### Arrow IPC / Feather

- **Arrow IPC (Feather v2)** read/write support ([`0fa35b2`](https://github.com/Dicklesworthstone/frankenpandas/commit/0fa35b299431e523e9e77e5f12458816f1ddb72c), [`bab40f8`](https://github.com/Dicklesworthstone/frankenpandas/commit/bab40f83ce650be49334df261244c415fab5d9d2) -- 2026-03-15)

### I/O Bug Fixes

- 3 bugs found in IO/conformance code review ([`5f0dd0b`](https://github.com/Dicklesworthstone/frankenpandas/commit/5f0dd0b7e2f64e1af2581151a2c4fa324b7e6b8f) -- 2026-03-15)
- Exactly-representable float in JSON round-trip test ([`828c0ab`](https://github.com/Dicklesworthstone/frankenpandas/commit/828c0abe18a648fae19a46cb1b24dd1958f6226e) -- 2026-02-26)
- Rename misleading adversarial test name ([`0dbeb31`](https://github.com/Dicklesworthstone/frankenpandas/commit/0dbeb31567367c8c00026a644eed5633a6434e25) -- 2026-03-15)

---

## Datetime and Timezone

### Datetime Parsing

- **`pd.to_datetime()`** equivalent for flexible datetime string parsing ([`e5557ec`](https://github.com/Dicklesworthstone/frankenpandas/commit/e5557ec453f8da195223601372824f5ed5b67ccc) -- 2026-03-16)
- **`pd.to_timedelta()`** and `timedelta_total_seconds()` ([`885bac2`](https://github.com/Dicklesworthstone/frankenpandas/commit/885bac292e34885981cd63170130a05278bd4b8a) -- 2026-03-16)
- Negative timedelta round-trip bug fix in `parse_hms` ([`89d81ab`](https://github.com/Dicklesworthstone/frankenpandas/commit/89d81ab664228a5c6e9df0d2bde21b3b57fc05a8) -- 2026-03-16)

### DatetimeAccessor

- **`dt` accessor** (DatetimeAccessor) for Series ([`d3fb430`](https://github.com/Dicklesworthstone/frankenpandas/commit/d3fb430a981d9bf0a3d8a84942dfa047f0a6247c) -- 2026-02-26)
- `dt.ceil`, `dt.floor` ([`82229b0`](https://github.com/Dicklesworthstone/frankenpandas/commit/82229b0ebf2a7d6a76bb1768e70e732fc2771dd4) -- 2026-03-04)
- `dt.round` ([`fc0f0a6`](https://github.com/Dicklesworthstone/frankenpandas/commit/fc0f0a6c7e8944883411d9a85e5e185a1a0d73af) -- 2026-03-04)
- `dt.to_timestamp` ([`bc27b5f`](https://github.com/Dicklesworthstone/frankenpandas/commit/bc27b5f53b6bdea260ab814867a50c3bdf9c7f68) -- 2026-03-04)

### Timezone Handling

- **chrono-tz support** and complete timezone handling rewrite ([`816519b`](https://github.com/Dicklesworthstone/frankenpandas/commit/816519b0822e2933a7a6e3a6fccd7eac8d6f10df) -- 2026-03-12)
- Series-level ambiguous/nonexistent timezone localization policies ([`add04cc`](https://github.com/Dicklesworthstone/frankenpandas/commit/add04cc116029095ff8bdff02cc49fd3c97cf024) -- 2026-03-12)
- Timezone operations and `DataFrame.from_tuples_with_index` ([`92d599c`](https://github.com/Dicklesworthstone/frankenpandas/commit/92d599c42c7e1b2c7ab8d4d6b75b41c1ff83b8eb) -- 2026-03-04)

---

## Conformance Testing

Differential testing against a pandas oracle to ensure semantic parity. The `fp-conformance`
crate runs FrankenPandas operations side-by-side with pandas and compares results via
machine-readable parity reports.

### Harness Infrastructure

- **Differential conformance harness** with pandas parity testing ([`59731b8`](https://github.com/Dicklesworthstone/frankenpandas/commit/59731b8067797a58ac386f58e081f0ad00995a00) -- 2026-02-14)
- CI gate forensics infrastructure with machine-readable reports ([`3e8c2b7`](https://github.com/Dicklesworthstone/frankenpandas/commit/3e8c2b7bce4f50f3a89a975ae605578b3c467188) -- 2026-02-14)
- Dynamic conformance packet-coverage tests ([`ebf9e0f`](https://github.com/Dicklesworthstone/frankenpandas/commit/ebf9e0f41667a0fcb3a8e669aa02e2c430e0939f) -- 2026-02-17)
- Fall back to fixture expectations when live oracle is unavailable ([`c29188b`](https://github.com/Dicklesworthstone/frankenpandas/commit/c29188bbcae1a0d2629bb9644b8a217c22530854) -- 2026-02-16)
- Capture oracle errors as report mismatches instead of propagating ([`90bffe2`](https://github.com/Dicklesworthstone/frankenpandas/commit/90bffe2c5bf28cd1f4ee688274ab9c838fc33854) -- 2026-02-16)
- Harden pandas import fallback, add `series_filter`/`series_head` ops ([`9a9535c`](https://github.com/Dicklesworthstone/frankenpandas/commit/9a9535c549f9c092f2fd14a758776a75e89e8a2c) -- 2026-02-16)
- Pandas oracle fixture generator ([`187ebab`](https://github.com/Dicklesworthstone/frankenpandas/commit/187ebab9b6ad2f2321d55d649b5d5441adb37a8f) -- 2026-02-20)
- `--python-bin` CLI flag for conformance runner, join benchmark binary ([`45bb12b`](https://github.com/Dicklesworthstone/frankenpandas/commit/45bb12b8ae3f361fe3ab7236984fdd257978ed79) -- 2026-03-03)
- End-to-end pandas workflow integration tests ([`dd20bae`](https://github.com/Dicklesworthstone/frankenpandas/commit/dd20bae717caa109c9e19e286e657e42a3b4f924) -- 2026-03-16)

### Conformance Packets

Over 55 conformance packets (FP-P2C and FP-P2D series) test specific pandas behaviors:

- **FP-P2C-006..011:** Compat-closure evidence packs covering iloc, loc, filter, groupby agg ([`8788d86`](https://github.com/Dicklesworthstone/frankenpandas/commit/8788d864b13c850f63cb868b2ad74fbd2952689b), [`962d9d6`](https://github.com/Dicklesworthstone/frankenpandas/commit/962d9d6a487a7812e3267c78986f045a150459d6) -- 2026-02-15/17)
- **FP-P2D-014..016:** DataFrame merge/concat, NaN aggregation, CSV round-trip ([`1aa7aba`](https://github.com/Dicklesworthstone/frankenpandas/commit/1aa7aba7e7b2fe912cdb4815765f3107c3d51e9a) -- 2026-02-17)
- **FP-P2D-017..024:** Series/DataFrame constructor paths, dtype/copy parity, shape taxonomy, dtype-spec normalization ([`9fcb157`](https://github.com/Dicklesworthstone/frankenpandas/commit/9fcb1572af27883b44a0daef106ff554d6c96686), [`bdc107f`](https://github.com/Dicklesworthstone/frankenpandas/commit/bdc107ff11f88f9723bdd951b65134586f87199c), [`bd909fd`](https://github.com/Dicklesworthstone/frankenpandas/commit/bd909fdf7b20f49af965570b2e4990c5481c34bc), [`7aef5b6`](https://github.com/Dicklesworthstone/frankenpandas/commit/7aef5b6a0acbd8bad2d05331d2d9e67718077ca0) -- 2026-02-17)
- **FP-P2D-025..027:** Dataframe selector, head-tail parity, negative-n head/tail ([`8d8993b`](https://github.com/Dicklesworthstone/frankenpandas/commit/8d8993bb1cabeeb6b92e65b1dfceef44e16e6f07), [`7477ad7`](https://github.com/Dicklesworthstone/frankenpandas/commit/7477ad7c3e0342d5d45ead4cb7fd8e1f78f1a072) -- 2026-02-17)
- **FP-P2D-028..032:** DataFrame concat axis=0/1 inner/outer join, column-order parity ([`4859714`](https://github.com/Dicklesworthstone/frankenpandas/commit/48597143f8d2f4fbfd18e57822c73a52f2ea378c)...[`d737635`](https://github.com/Dicklesworthstone/frankenpandas/commit/d7376350a378ac4fb35f99d1c6930b9fda1025cb) -- 2026-02-17/18)
- **FP-P2D-035..039:** Merge validate, indicator, suffix, sort, cross-join parity ([`c0007a0`](https://github.com/Dicklesworthstone/frankenpandas/commit/c0007a088003724939c234119f98e43134b6e55a)...[`6ccbc1a`](https://github.com/Dicklesworthstone/frankenpandas/commit/6ccbc1ab9388ee7db2c14dc2caef5a6bc79acb8a) -- 2026-02-18)
- **FP-P2D-040..041:** Sort and any/all oracle-backed fixture packets ([`f41e0b0`](https://github.com/Dicklesworthstone/frankenpandas/commit/f41e0b0b65e6259545e06e67264877b5db6a515c) -- 2026-02-19)
- **FP-P2D-053..055:** set/reset_index, duplicate detection, series arithmetic ([`6f2180c`](https://github.com/Dicklesworthstone/frankenpandas/commit/6f2180c59bef60c8a5d31a2f72496f377c9d5c2a), [`fda6a0a`](https://github.com/Dicklesworthstone/frankenpandas/commit/fda6a0af057162c2d25ea7c639b0e2ee54458a29) -- 2026-02-20)

### Property-Based and Adversarial Tests

- DataFrame arithmetic/comparison property tests ([`3ba5d5d`](https://github.com/Dicklesworthstone/frankenpandas/commit/3ba5d5d64b5d9ee6619c6bf7b559a171b3348358) -- 2026-03-15)
- Join/filter performance baselines ([`fc42c5e`](https://github.com/Dicklesworthstone/frankenpandas/commit/fc42c5e4d278be909824eb5a226207e3a8d293b1) -- 2026-03-15)
- Property-based IO round-trip tests for SQL, Excel, Feather ([`0b44cbe`](https://github.com/Dicklesworthstone/frankenpandas/commit/0b44cbe2f814c35fca653f8723728c9eb7593907), [`e51fa32`](https://github.com/Dicklesworthstone/frankenpandas/commit/e51fa32ab66645d4650002048f958d19af7ab4f9) -- 2026-03-15)
- 15 adversarial parser tests for CSV/JSON/SQL ([`4835189`](https://github.com/Dicklesworthstone/frankenpandas/commit/4835189a0b0350139332b36b0e1c4a02260f3028) -- 2026-03-15)
- eval/query expression parity tests ([`6a9a854`](https://github.com/Dicklesworthstone/frankenpandas/commit/6a9a85433d3ecbc6d1d4140d95c8c235fe62ea11) -- 2026-03-15)
- merge_asof edge-case tests ([`c853e97`](https://github.com/Dicklesworthstone/frankenpandas/commit/c853e9755d2fa0a8403becb7430b6fa44cdcce00) -- 2026-03-15)

### Conformance Bug Fixes

- Reject sidecars with envelope/packet count mismatches ([`30f780f`](https://github.com/Dicklesworthstone/frankenpandas/commit/30f780faf199a62829d62a53ebf8e44a76e9483c) -- 2026-02-16)
- Enforce sidecar `artifact_id` packet coherence ([`4308f4b`](https://github.com/Dicklesworthstone/frankenpandas/commit/4308f4bc4895af14f736e7206b1ec43d7fab1f69) -- 2026-02-16)
- Cap decode proofs and enforce sidecar bounds ([`2eb9729`](https://github.com/Dicklesworthstone/frankenpandas/commit/2eb97298106c87664b21d7d93ea3a02f1b317139) -- 2026-02-16)

---

## Runtime and Governance

The `fp-runtime` crate provides strict/hardened admission policies and the `EvidenceLedger`
for auditable execution traces.

- **ASUPERSYNC governance gates**, property tests, and runtime skeleton ([`3013c90`](https://github.com/Dicklesworthstone/frankenpandas/commit/3013c9053c24b120001fe0707872ff57649fe37c), [`6dbfba6`](https://github.com/Dicklesworthstone/frankenpandas/commit/6dbfba61e315718bba01394aa05c1a985c818fa0) -- 2026-02-14)
- Harden prior bounds and sync placeholder scrub semantics ([`3997fbb`](https://github.com/Dicklesworthstone/frankenpandas/commit/3997fbb9067419966057a43f4ca2e5923a6c22d4) -- 2026-02-16)
- RaptorQ placeholder scrub status changed from "ok" to "placeholder" ([`b675bd5`](https://github.com/Dicklesworthstone/frankenpandas/commit/b675bd5172c610b1b7b1c04e319fd85beebf492a) -- 2026-02-16)
- COMPAT-CLOSURE threat model and Rust integration plan ([`4895226`](https://github.com/Dicklesworthstone/frankenpandas/commit/4895226bd2a0eeabbc9b1a85b998b862c94c0496) -- 2026-02-15)
- HyperLogLog rho calculation: use sentinel bit instead of unconditional OR ([`e39bc95`](https://github.com/Dicklesworthstone/frankenpandas/commit/e39bc95e9167bef7d1daaf4058bded19c67901aa) -- 2026-02-21)
- Char boundary panic fix in governance gate ([`54cef39`](https://github.com/Dicklesworthstone/frankenpandas/commit/54cef3982d84068f232ac02638ed0531d0c5cbcc) -- 2026-02-20)

---

## FrankenTUI (Experimental)

Terminal UI dashboard for interactive data exploration. Scaffold-stage crate.

- Crate scaffold with workspace integration ([`39a56dc`](https://github.com/Dicklesworthstone/frankenpandas/commit/39a56dc65f869633c4da94a27822be4d43e4a0e5) -- 2026-02-15)
- CLI binary entry point ([`020e198`](https://github.com/Dicklesworthstone/frankenpandas/commit/020e1986dc937adbf4aa5cf989ac7139e37aa901) -- 2026-02-15)
- TUI integration library (387 lines) ([`25c5dd4`](https://github.com/Dicklesworthstone/frankenpandas/commit/25c5dd48404d469e54db1299ac5e9681c8b2af23) -- 2026-02-15)
- E2E scenario replay and differential validation ([`104997e`](https://github.com/Dicklesworthstone/frankenpandas/commit/104997e4b74e3eee69f59509691b94c27cb9eb89) -- 2026-02-15)

---

## Performance Optimization

### Optimization Rounds (2026-02-13)

Evidence-driven optimization with formal isomorphism proofs and baseline/opportunity matrices.

- **Round 3:** Identity-alignment fast path ([`e30f4a4`](https://github.com/Dicklesworthstone/frankenpandas/commit/e30f4a410d6e911c29a508392118a769c75eaaf9))
- **Round 3:** Dense Int64 groupby ([`5ab34f8`](https://github.com/Dicklesworthstone/frankenpandas/commit/5ab34f8f9d058d75024772756e54d8cc722c7e08))
- **Round 3:** Lazy `has_duplicates` memoization ([`5ab34f8`](https://github.com/Dicklesworthstone/frankenpandas/commit/5ab34f8f9d058d75024772756e54d8cc722c7e08))
- **Round 4-5:** Additional optimization evidence artifacts ([`6b00e16`](https://github.com/Dicklesworthstone/frankenpandas/commit/6b00e1600ffa44391ad7ba518c3fdeb1eef57f3a), [`ede5c3e`](https://github.com/Dicklesworthstone/frankenpandas/commit/ede5c3e9170ef9812aee6e2051428c8ff658eafb))

### Hot-Path Fixes

- Eliminate redundant identity casts in scalar coercion hot path ([`8a53e9a`](https://github.com/Dicklesworthstone/frankenpandas/commit/8a53e9a4d7b15c39351b5ea93960936dd12ccfad) -- 2026-02-13)
- Bumpalo arena allocator path for join intermediate vectors ([`d31a744`](https://github.com/Dicklesworthstone/frankenpandas/commit/d31a744376e0f50036021885c2b1c048292d81dc) -- 2026-02-14)

---

## Licensing and Project Metadata

- **Initial public import** of FrankenPandas clean-room Rust port ([`4e35648`](https://github.com/Dicklesworthstone/frankenpandas/commit/4e35648e4c349032ba7e2ce4604bf4cabde9f774) -- 2026-02-13)
- Initialize workspace with phase2c framework ([`b2d967a`](https://github.com/Dicklesworthstone/frankenpandas/commit/b2d967a81d8ae9e82d161a2d66ea567a22952e8b) -- 2026-02-13)
- **MIT + OpenAI/Anthropic rider** adopted across workspace and all crates ([`a9889ca`](https://github.com/Dicklesworthstone/frankenpandas/commit/a9889cafc70ec04293d907d06f0d80868263e4e8) -- 2026-02-18)
- Feature Parity Mandate for complete pandas API coverage ([`63cf641`](https://github.com/Dicklesworthstone/frankenpandas/commit/63cf641500a42a90b84541a972af5fe81c5ede24) -- 2026-02-13)
- Bump asupersync dependency from v0.1.1 to v0.2.0 ([`b9684f7`](https://github.com/Dicklesworthstone/frankenpandas/commit/b9684f740237a0156cd169003b5be118e7436e36) -- 2026-02-15)
- Complete README.md rewrite to 1574 lines ([`81c58c4`](https://github.com/Dicklesworthstone/frankenpandas/commit/81c58c4e2c3b7d9d303b74d9733932cefff62425)...[`95bc05e`](https://github.com/Dicklesworthstone/frankenpandas/commit/95bc05e04b60cbe13ba07938d6ef4bec32fb154c) -- 2026-03-17)

---

## Commit Statistics

| Metric | Value |
|--------|-------|
| Total commits | 269 |
| Date range | 2026-02-13 to 2026-03-17 |
| Tags / releases | None (pre-release) |
| Workspace crates | 12 |
| Lines of Rust | 84,000+ |
| Public functions | 788 |
| IO formats | 7 |
| Conformance packets | 55+ |
| License | MIT + OpenAI/Anthropic rider |
