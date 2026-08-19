# Changelog

All notable changes to FrankenPandas are documented in this file, organized by capability area.

FrankenPandas is a clean-room Rust reimplementation of the full pandas API surface:
**12+ workspace crates, `#![forbid(unsafe_code)]` workspace-wide**. Workspace version is **0.2.0**.

Repository: <https://github.com/Dicklesworthstone/frankenpandas>

Scope window: project inception on 2026-02-13 through HEAD on 2026-08-19.
This 2026-08-19 refresh covers the previously undocumented window **2026-05-17 through 2026-08-19**
(Phase 3 plus published GitHub Releases). Phase 1 (thematic sections) and Phase 2 are unchanged.
The 2026-05-16 commit-statistics / open-workstreams block is a **snapshot**, not current state.
Representative commits in each wave are live-linked to GitHub.

## Version Timeline

`Kind` distinguishes a published GitHub Release from a plain git tag. Crate-level tags
(`fp-types-v0.2.0`, …) exist alongside the workspace tags below; they are not listed row-by-row.

| Version | Kind | Date | Summary |
|---------|------|------|---------|
| `[Unreleased]` / Phase 3 | dev head | 2026-05-17 → 2026-08-19 | 5,077 non-merge commits after the 2026-05-16 snapshot: `fp-python` bindings, measured vs-pandas kernels, radix sorts, groupby rolling scatter, oracle/parity tail, repo-janitor docs move. |
| [`frankenpandas-v0.2.0`](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/frankenpandas-v0.2.0) | GitHub Release | 2026-07-28 | Workspace 0.2.0 crate release (with matching `fp-*-v0.2.0` crate Releases). |
| [`v0.1.2`](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/v0.1.2) | GitHub Release | 2026-06-22 | Workspace v0.1.2 (also [`frankenpandas-v0.1.2`](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/frankenpandas-v0.1.2)). |
| [`v0.1.1`](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/v0.1.1) | GitHub Release | 2026-06-21 | Workspace v0.1.1 (also [`frankenpandas-v0.1.1`](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/frankenpandas-v0.1.1)). |
| [`fp-bench-v0.1.0`](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/fp-bench-v0.1.0) | GitHub Release | 2026-06-15 | First `fp-bench` crate Release. |
| [`fp-python-v0.1.0`](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/fp-python-v0.1.0) | GitHub Release | 2026-05-25 | First `fp-python` crate Release. |
| [`frankenpandas-v0.1.0`](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/frankenpandas-v0.1.0) | GitHub Release | 2026-04-23 | First facade-crate Release (crate tags `fp-types-v0.1.0` … `fp-frankentui-v0.1.0` the same day). |

Tags `backup/local-main-pre-rebase-80a36bf9` and `coda-fpindex-autostash-backup` are **not Releases**.

(A legacy compatibility branch diverges at [`a9889ca`](https://github.com/Dicklesworthstone/frankenpandas/commit/a9889cafc70ec04293d907d06f0d80868263e4e8)
— MIT licensing, 2026-02-18 — and stays in sync with `main` after every push.)

This changelog is organized in four layers:

1. **Phase 1 (capability-foundation era, 2026-02-13 → 2026-03-17)** — the original
   per-subsystem thematic sections below: Core Data Engine, DataFrame API, Series API,
   GroupBy, Window Functions, Join / Merge / Concat, Expression Engine, Index /
   MultiIndex, I/O, Datetime / Timezone, Conformance Testing, Runtime / Governance,
   FrankenTUI, Performance, Licensing.
2. **Phase 2 (pandas-parity completion era, 2026-03-18 → 2026-05-16)** — three
   sub-phases (Phase 2a, Phase 2b, Phase 2c).
3. **Phase 3 (measured-kernel + Python-bindings era, 2026-05-17 → 2026-08-19)** — dated
   capability waves plus the 2026-08-18/19 repo-janitor docs reorganization.
4. **Commit Statistics + Open Workstreams** — 2026-05-16 snapshot (not re-audited here).

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
- [Phase 2: Pandas-Parity Completion Era](#phase-2-pandas-parity-completion-era-2026-03-18--2026-05-16)
- [Phase 3: Measured kernels, Python bindings, published releases](#phase-3-measured-kernels-python-bindings-published-releases-2026-05-17--2026-08-19)

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

## Phase 2: Pandas-Parity Completion Era (2026-03-18 → 2026-05-16)

The capability waves before this point built the foundation. Phase 2 is
about closing the long tail of pandas semantics, finishing the IO
surface, hardening every reduction and accessor against null/dtype/Utf8
edge cases, and pushing the differential conformance harness from a few
hundred packets to over a thousand fixture-backed packets plus a live
pandas oracle running in CI on every PR. Roughly **2,500 commits** land
in this window across three sub-phases below.

### Phase 2a: Conformance harness era (2026-03-18 → 2026-04-15)

The expression engine grew real arithmetic. DataFrame finally became
fully two-dimensional (axis=1 everywhere). The index alignment planner
learned to handle duplicate labels correctly. And a typed group-key
infrastructure replaced the Debug-string hashing path that had been
quietly miscomparing keys across the codebase.

#### Arithmetic operator expansion (mod / pow / floordiv)

- **`fp-expr` learns Modulo / FloorDiv / Pow AST nodes** with correct precedence (`**` > unary > `*`/`/`/`//`/`%`), leading-dot floats, and bitwise shorthand `&|~` ([`0cc00cca`](https://github.com/Dicklesworthstone/frankenpandas/commit/0cc00cca) -- 2026-04-09)
- **`fp-columnar`** gains vectorized `f64` execution for these ops; **`int64` preserved** when divisors are non-zero ([`d5c6bc81`](https://github.com/Dicklesworthstone/frankenpandas/commit/d5c6bc81) -- 2026-04-09)
- Overflow guards land for `i64` mod/floordiv ([`a68f0757`](https://github.com/Dicklesworthstone/frankenpandas/commit/a68f0757) -- 2026-04-10)

#### Axis-1 / row-wise DataFrame operations

- **DataFrame `diff_axis1`, `pct_change_axis1`, `shift_axis1`, `rank` axis=1, `take_rows` / `take_columns`** ([`56055711`](https://github.com/Dicklesworthstone/frankenpandas/commit/56055711) -- 2026-04-11)
- **`DataFrame::melt`** with mixed-type numeric promotion ([`eddec82d`](https://github.com/Dicklesworthstone/frankenpandas/commit/eddec82d) -- 2026-04-11)
- Row-wise `apply_rows` and Series `apply`/`map_func` ([`af40a5db`](https://github.com/Dicklesworthstone/frankenpandas/commit/af40a5db) -- 2026-04-12)

#### Typed group-key infrastructure (`ScalarKey`)

- **Replaces debug-string group keys** (`format!("{val:?}")`) with a typed `ScalarKey` across fp-frame groupby, `mode`, `describe`, `pivot`, `pivot_table`, and categorical APIs — fixes a long tail of subtle parity bugs from Debug-formatted hashing ([`1b52ae43`](https://github.com/Dicklesworthstone/frankenpandas/commit/1b52ae43) -- 2026-04-11)
- Float zero (`-0.0` vs `0.0`) normalized in groupby keys, `nannunique`, and frame uniqueness paths ([`c0a94b44`](https://github.com/Dicklesworthstone/frankenpandas/commit/c0a94b44), [`8da6621e`](https://github.com/Dicklesworthstone/frankenpandas/commit/8da6621e), [`cfab29a3`](https://github.com/Dicklesworthstone/frankenpandas/commit/cfab29a3) -- 2026-04-15)

#### Index alignment for non-unique / duplicate labels

- **`align_union` routed through `align_non_unique`** when either side has duplicates ([`ed3e3c79`](https://github.com/Dicklesworthstone/frankenpandas/commit/ed3e3c79) -- 2026-04-15)
- `Series::align`, `DataFrame::align_on_index`, `filter_rows` all gain duplicate-label semantics ([`8b72c505`](https://github.com/Dicklesworthstone/frankenpandas/commit/8b72c505) -- 2026-04-15)

#### `to_datetime` parity (units, origins, timezones)

- Pandas-compatible **`to_datetime` unit handling, UTC + origin parity, julian and numeric origins** ([`911902c3`](https://github.com/Dicklesworthstone/frankenpandas/commit/911902c3), [`ef58de21`](https://github.com/Dicklesworthstone/frankenpandas/commit/ef58de21), [`da961836`](https://github.com/Dicklesworthstone/frankenpandas/commit/da961836) -- 2026-04-10)
- Normalize timezone-aware to_datetime strings ([`8f8e3faa`](https://github.com/Dicklesworthstone/frankenpandas/commit/8f8e3faa) -- 2026-04-11)

#### `merge_asof` feature completeness

- **`tolerance`, `by`, `allow_exact_matches`** parameters ([`1ce16699`](https://github.com/Dicklesworthstone/frankenpandas/commit/1ce16699) -- 2026-04-10)
- Direction validation, suffix handling, and right-side dtype preservation in output ([`19d7df69`](https://github.com/Dicklesworthstone/frankenpandas/commit/19d7df69), [`10fcf3e8`](https://github.com/Dicklesworthstone/frankenpandas/commit/10fcf3e8) -- 2026-04-12)

#### IO hardening (CSV / JSON / Parquet / Excel)

- **CSV `usecols`, `skiprows`, `dtype` coercion validation** + pandas-default NA value handling ([`a62f04b8`](https://github.com/Dicklesworthstone/frankenpandas/commit/a62f04b8), [`c6b2c93e`](https://github.com/Dicklesworthstone/frankenpandas/commit/c6b2c93e) -- 2026-04-13)
- **Duplicate column rejection** in readers ([`fea96177`](https://github.com/Dicklesworthstone/frankenpandas/commit/fea96177) -- 2026-04-14)
- JSON column/index order preservation ([`1f59cbe1`](https://github.com/Dicklesworthstone/frankenpandas/commit/1f59cbe1), [`c1e8bc0d`](https://github.com/Dicklesworthstone/frankenpandas/commit/c1e8bc0d) -- 2026-04-14)
- CSV skiprows semantics fix ([`1f9db747`](https://github.com/Dicklesworthstone/frankenpandas/commit/1f9db747) -- 2026-04-14)

### Phase 2b: Massive parity build-out (2026-04-16 → 2026-05-01)

A 14-day, ~1,500-commit sprint. Twelve crates expand in parallel: the
SQL backend gets a full `SqlConnection` + `SqlInspector` trait surface
with 70+ sub-beads, the row MultiIndex epic ships, hundreds of live
pandas-oracle conformance tests land, fuzz infrastructure grows by 90+
targets, and a 19-pass review-mode audit drives 14 documented
divergences plus dozens of dtype/null/Utf8/edge-case fixes.

#### `fp-columnar` Column-API parity sweep (br-frankenpandas-w3ji → -dklr)

- **~30 commits in 24 hours on 2026-04-21**: take/slice/concat/repeat, head/tail/cumulative/unique, abs/shift/clip/round/isin, value_counts, sort/argsort/diff/duplicated/between, aggregation helpers, where/mask, nlargest/nsmallest, astype, rank+searchsorted, quantile/mode/memory_usage, interpolate/drop_duplicates/compare/map, argmin/argmax/is_monotonic/combine_first/clip_lower/clip_upper, describe/combine/apply_float/hist_counts, nunique/any/all/is_unique/has_duplicates/pct_change, isnull/notnull/var/std/sem/skew/kurt/ptp, rolling_window_sum, diff_valid/sample/first_valid/last_valid, where_cond_series/mask_series/replace_values/nonzero, and the full `ValidityMask` surface ([`593b6792`](https://github.com/Dicklesworthstone/frankenpandas/commit/593b6792), [`1daf6f53`](https://github.com/Dicklesworthstone/frankenpandas/commit/1daf6f53), [`38d15b55`](https://github.com/Dicklesworthstone/frankenpandas/commit/38d15b55), [`81a68a91`](https://github.com/Dicklesworthstone/frankenpandas/commit/81a68a91), [`f6beeedb`](https://github.com/Dicklesworthstone/frankenpandas/commit/f6beeedb))

#### `fp-types` — proper `Timestamp` / `Timedelta` / `Period` / `Interval` types

- **`Timedelta`** nanosecond struct ([`c74eec7c`](https://github.com/Dicklesworthstone/frankenpandas/commit/c74eec7c)), **`TimedeltaIndex` + `timedelta_range`** ([`6c128046`](https://github.com/Dicklesworthstone/frankenpandas/commit/6c128046)), Timedelta arithmetic ([`68e2087c`](https://github.com/Dicklesworthstone/frankenpandas/commit/68e2087c))
- **`Timestamp`** struct + Timedelta arithmetic ([`574b4b57`](https://github.com/Dicklesworthstone/frankenpandas/commit/574b4b57)); `floor_to`/`ceil_to`/`round_to` ([`e0851273`](https://github.com/Dicklesworthstone/frankenpandas/commit/e0851273)); string-unit rounding + `unit_to_nanos` ([`6f9d199d`](https://github.com/Dicklesworthstone/frankenpandas/commit/6f9d199d))
- **`Interval` + `IntervalClosed`** scaffolding ([`6e8ec436`](https://github.com/Dicklesworthstone/frankenpandas/commit/6e8ec436)); `interval_range` builders ([`a01b0cad`](https://github.com/Dicklesworthstone/frankenpandas/commit/a01b0cad))
- **`Period` + `PeriodFreq`** scaffolding ([`b646d880`](https://github.com/Dicklesworthstone/frankenpandas/commit/b646d880)); `period_range` builder ([`6417afe0`](https://github.com/Dicklesworthstone/frankenpandas/commit/6417afe0))
- NanOps parity for `nancumsum`/`nancumprod`/`nancummax`/`nancummin`/`nanquantile`/`nanarg*` ([`84ce16ae`](https://github.com/Dicklesworthstone/frankenpandas/commit/84ce16ae), [`e2d43fac`](https://github.com/Dicklesworthstone/frankenpandas/commit/e2d43fac))

#### SQL backend Phase 1 — generic `SqlConnection` + `SqlInspector` (frankenpandas-fd90, ~70 sub-beads)

- **Generic SQL connection foundation** ([`65df0486`](https://github.com/Dicklesworthstone/frankenpandas/commit/65df0486)); sqlite backend gated behind `sql-sqlite` feature ([`bf14836f`](https://github.com/Dicklesworthstone/frankenpandas/commit/bf14836f))
- `SqlConnection` capability + dialect probes ([`d87a3a66`](https://github.com/Dicklesworthstone/frankenpandas/commit/d87a3a66)); overridable `quote_identifier` ([`86fd5f1d`](https://github.com/Dicklesworthstone/frankenpandas/commit/86fd5f1d)); `SqlReadOptions::dtype` ([`5131382b`](https://github.com/Dicklesworthstone/frankenpandas/commit/5131382b))
- **Schema introspection probes**, cross-schema read/write, schema-qualified DROP ([`006938b6`](https://github.com/Dicklesworthstone/frankenpandas/commit/006938b6), [`8ac537a1`](https://github.com/Dicklesworthstone/frankenpandas/commit/8ac537a1), [`4af093c7`](https://github.com/Dicklesworthstone/frankenpandas/commit/4af093c7))
- **`SqlConnection::list_tables`/`table_schema`/`list_schemas`/`truncate_table`/`server_version`/`primary_key_columns`/`max_identifier_length`/`list_indexes`/`list_foreign_keys`/`list_views`/`list_unique_constraints`/`table_comment`** ([`2955c49e`](https://github.com/Dicklesworthstone/frankenpandas/commit/2955c49e), [`fbfd2d7e`](https://github.com/Dicklesworthstone/frankenpandas/commit/fbfd2d7e), [`92642d0f`](https://github.com/Dicklesworthstone/frankenpandas/commit/92642d0f), [`ca90ed6d`](https://github.com/Dicklesworthstone/frankenpandas/commit/ca90ed6d), [`87808946`](https://github.com/Dicklesworthstone/frankenpandas/commit/87808946), [`d10ecb7e`](https://github.com/Dicklesworthstone/frankenpandas/commit/d10ecb7e), [`6fd36a0f`](https://github.com/Dicklesworthstone/frankenpandas/commit/6fd36a0f), [`1ce78bdb`](https://github.com/Dicklesworthstone/frankenpandas/commit/1ce78bdb), [`eb343bf2`](https://github.com/Dicklesworthstone/frankenpandas/commit/eb343bf2), [`9800a117`](https://github.com/Dicklesworthstone/frankenpandas/commit/9800a117), [`8b2a763f`](https://github.com/Dicklesworthstone/frankenpandas/commit/8b2a763f), [`a300340e`](https://github.com/Dicklesworthstone/frankenpandas/commit/a300340e))
- **Chunked SQL reads** + indexed SQL chunks ([`28c8d619`](https://github.com/Dicklesworthstone/frankenpandas/commit/28c8d619), [`90fd5498`](https://github.com/Dicklesworthstone/frankenpandas/commit/90fd5498)..[`48c06e18`](https://github.com/Dicklesworthstone/frankenpandas/commit/48c06e18))
- **`SqlInspector` wrapper** for unified introspection (SQLAlchemy-shaped) ([`032c6aac`](https://github.com/Dicklesworthstone/frankenpandas/commit/032c6aac), [`1ea88759`](https://github.com/Dicklesworthstone/frankenpandas/commit/1ea88759), [`214cb795`](https://github.com/Dicklesworthstone/frankenpandas/commit/214cb795)); `reflect_all_tables`/`reflect_all_views` ([`99d39fee`](https://github.com/Dicklesworthstone/frankenpandas/commit/99d39fee), [`9c689557`](https://github.com/Dicklesworthstone/frankenpandas/commit/9c689557))

#### Row MultiIndex epic (br-frankenpandas-1zzp + 6 child slices)

- **Closed 2026-04-22**: long-running row-MultiIndex gap shipped via six cohesive slices
- DataFrame `row_multiindex` field ([`8f63166e`](https://github.com/Dicklesworthstone/frankenpandas/commit/8f63166e)); groupby emits real row MultiIndex on multi-key ([`7a503aeb`](https://github.com/Dicklesworthstone/frankenpandas/commit/7a503aeb))
- Tuple lookup + `xs` APIs ([`5295c03c`](https://github.com/Dicklesworthstone/frankenpandas/commit/5295c03c)); reshape round-trips ([`9c3e5eca`](https://github.com/Dicklesworthstone/frankenpandas/commit/9c3e5eca)); IO round-trip across formats ([`22c7c3f9`](https://github.com/Dicklesworthstone/frankenpandas/commit/22c7c3f9)); live-oracle conformance ([`d35ab20c`](https://github.com/Dicklesworthstone/frankenpandas/commit/d35ab20c)). Epic closed via [`ba2e61db`](https://github.com/Dicklesworthstone/frankenpandas/commit/ba2e61db)
- **`MultiIndex.is_monotonic` + `is_lexsorted` predicates** ([`f4cf4e4c`](https://github.com/Dicklesworthstone/frankenpandas/commit/f4cf4e4c))

#### Live pandas-oracle conformance tests (~290 commits)

Hundreds of `test(fp-conformance): live pandas oracle for X` entries across Series, DataFrame, GroupBy, Index, str/dt accessors, rolling/expanding/ewm, IO, asof/at_time/between_time, compare, str.normalize/casefold (Unicode), `nan_*` primitives, categorical, etc. — drives the live-oracle suite from a handful into the **hundreds**. Reps: [`7d37afac`](https://github.com/Dicklesworthstone/frankenpandas/commit/7d37afac), [`f3b2e934`](https://github.com/Dicklesworthstone/frankenpandas/commit/f3b2e934), [`dd0b6e25`](https://github.com/Dicklesworthstone/frankenpandas/commit/dd0b6e25), [`ff2ca9b5`](https://github.com/Dicklesworthstone/frankenpandas/commit/ff2ca9b5), [`a76ca9f2`](https://github.com/Dicklesworthstone/frankenpandas/commit/a76ca9f2). **CI gate enforced**: `fix(conformance): require live oracle in CI` ([`9c118947`](https://github.com/Dicklesworthstone/frankenpandas/commit/9c118947)).

#### Conformance fixture packets FP-P2D-064 → FP-P2D-433

Massive expansion of the deterministic fixture-packet ledger: cumulative ops, str accessor lower/upper/title/swapcase/zfill/center/pad, dt accessor quarter/dayofyear/is_month_start/is_year_start/leap_year/strftime/floor/ceil/round/total_seconds/to_timestamp, rolling/expanding/ewm/resample, at_time/between_time/asof, DataFrame transpose/top-N/insert/assign/rename/reindex/drop/replace/where/mask/shift axis=1/describe/corr/cov/idxmin/idxmax/sem/skew/kurtosis/prod/sum/mean/std/var/min/max/median/any/all/nunique/quantile/value_counts/memory_usage. Null-hardened variants added across the board ([`b96d9e75`](https://github.com/Dicklesworthstone/frankenpandas/commit/b96d9e75)..[`8e818740`](https://github.com/Dicklesworthstone/frankenpandas/commit/8e818740)). Parity-gate manifests refreshed multiple times.

#### IO format surface expansion

- **CSV** options: `comment`, `on_bad_lines`, `decimal`, `true/false_values`, `parse_dates` (incl. dict-style rename), `thousands`, `quote/escape`, `skipfooter`, `lineterminator`, `index_label`, full write options struct ([`d47d3f5e`](https://github.com/Dicklesworthstone/frankenpandas/commit/d47d3f5e), [`44e779ee`](https://github.com/Dicklesworthstone/frankenpandas/commit/44e779ee), [`d29b2b93`](https://github.com/Dicklesworthstone/frankenpandas/commit/d29b2b93), [`eb456c9f`](https://github.com/Dicklesworthstone/frankenpandas/commit/eb456c9f), [`c19b622b`](https://github.com/Dicklesworthstone/frankenpandas/commit/c19b622b), [`4eb3e4e0`](https://github.com/Dicklesworthstone/frankenpandas/commit/4eb3e4e0), [`a28606f4`](https://github.com/Dicklesworthstone/frankenpandas/commit/a28606f4), [`483509bc`](https://github.com/Dicklesworthstone/frankenpandas/commit/483509bc), [`2d607c2a`](https://github.com/Dicklesworthstone/frankenpandas/commit/2d607c2a), [`9aa5cdb4`](https://github.com/Dicklesworthstone/frankenpandas/commit/9aa5cdb4), [`89603d45`](https://github.com/Dicklesworthstone/frankenpandas/commit/89603d45), [`9009ecaf`](https://github.com/Dicklesworthstone/frankenpandas/commit/9009ecaf))
- **Excel** index-label preservation roundtrip ([`ff69357a`](https://github.com/Dicklesworthstone/frankenpandas/commit/ff69357a)), headerless reads ([`d071612c`](https://github.com/Dicklesworthstone/frankenpandas/commit/d071612c)), `sheets_ordered` preserving workbook order ([`0f795b47`](https://github.com/Dicklesworthstone/frankenpandas/commit/0f795b47)), full `to_excel` option parity ([`e5af6e52`](https://github.com/Dicklesworthstone/frankenpandas/commit/e5af6e52))
- **Arrow IPC stream** surfaced as 8th IO format ([`7a1c9058`](https://github.com/Dicklesworthstone/frankenpandas/commit/7a1c9058))
- **JSON** `to_json("table")` with Table Schema ([`3fe14241`](https://github.com/Dicklesworthstone/frankenpandas/commit/3fe14241)); `to_dict("series")` ([`091691d6`](https://github.com/Dicklesworthstone/frankenpandas/commit/091691d6))
- Cross-format round-trip fuzz target ([`49f0cdf9`](https://github.com/Dicklesworthstone/frankenpandas/commit/49f0cdf9))

#### Index API parity (fp-index)

- `MultiIndex.get_indexer_non_unique` ([`67430d4c`](https://github.com/Dicklesworthstone/frankenpandas/commit/67430d4c)); `MultiIndex.isin` tuple/level parity ([`2f093294`](https://github.com/Dicklesworthstone/frankenpandas/commit/2f093294)); `MultiIndex.duplicated`/`is_unique` ([`057f28f4`](https://github.com/Dicklesworthstone/frankenpandas/commit/057f28f4))
- Index `insert`/`delete`/`append`/`repeat`/`dropna` ([`b5dbaf56`](https://github.com/Dicklesworthstone/frankenpandas/commit/b5dbaf56)); `equals`/`identical`/`value_counts`/`shift`/`any`/`all` ([`1c24f63f`](https://github.com/Dicklesworthstone/frankenpandas/commit/1c24f63f))
- `Index.to_list`/`format`/`putmask` ([`f68e2c03`](https://github.com/Dicklesworthstone/frankenpandas/commit/f68e2c03)); `Index.asof`/`searchsorted`/`memory_usage`/`nlevels` ([`b17e94ae`](https://github.com/Dicklesworthstone/frankenpandas/commit/b17e94ae))

#### Major fixes in this window

- **Pandas-error parity hardening**: reject empty separator in `str.split_expand`, `Column.combine` honors `fill_value=None`, `to_dict(orient='index')` rejects duplicate index, reject `table` JSON for MultiIndex columns, validate `Series.dt.to_timestamp` periods, `rolling min_periods=0` parity, `Expanding` empty-window NaN parity, `Series.unique` retains a single null marker, `Series.searchsorted` accepts non-numeric needles
- **Dtype / null-promotion parity**: preserve Int64/Bool dtype in `DataFrame.mode` pad cells, `semantic_eq` bridges all Null kinds, preserve Series `abs`/cumulative/round dtypes, accept legacy `"str"`/`"string"` serde aliases for Utf8, honor categorical ordering semantics
- **Numeric edge cases — infinity, modulo, divmod, rounding**: handle infinity in `python_mod_f64`, match pandas modulo signs, match pandas infinite-float divmod, reuse pandas divmod helpers, match pandas half-even rounding, prevent integer overflow in HyperLogLog estimate
- **NA sentinel + skipna alignment**: match pandas `argsort`/`argmin`/`argmax` NA sentinels, missing `searchsorted` needle, normalize JSON/JSONL nullable numerics
- **Tie-break / ordering**: preserve `value_counts` tie order, align `FP-P2C-005 groupby_sum_order` with pandas default sort, restore grouped-window fallback schema, align rolling-count null windows, harden Series top-k keep handling
- **Expression parser hardening**: escape sequences in string literals + malformed float detection, chained-comparison pairwise AND, backtick column parsing
- **Outer-join + concat parity** ([`0d99fd69`](https://github.com/Dicklesworthstone/frankenpandas/commit/0d99fd69), [`3bb41478`](https://github.com/Dicklesworthstone/frankenpandas/commit/3bb41478))
- **Sparse + Interval + Period correctness** in the compatibility surface ([`04827359`](https://github.com/Dicklesworthstone/frankenpandas/commit/04827359), [`49100a25`](https://github.com/Dicklesworthstone/frankenpandas/commit/49100a25), [`affd37fe`](https://github.com/Dicklesworthstone/frankenpandas/commit/affd37fe))

#### 19-pass cc-pandas review-mode session (2026-04-22 → 2026-04-23)

- A bv-style "review-only" audit pass yielded **58 filed beads** and **14 shipped fixes** (handoff at [`f69dad0c`](https://github.com/Dicklesworthstone/frankenpandas/commit/f69dad0c))
- 19 numbered triage passes (HIGH/MEDIUM/LOW gaps)
- Three subsequent reality-check passes surfaced **14 documented divergences** in `DISCREPANCIES.md` (DISC-001 through DISC-014; 2 RESOLVED in the dedicated "Resolved Divergences" section (DISC-005 + DISC-013); 12 active/INVESTIGATING/WILL-FIX/ACCEPTED)

#### Infrastructure (2026-04-22 → 2026-04-23 push)

- **CI restructure**: split monolithic `checks` job into fmt/lint/test/conformance/gates ([`f6427657`](https://github.com/Dicklesworthstone/frankenpandas/commit/f6427657)); cross-platform matrix + feature matrix ([`6a157919`](https://github.com/Dicklesworthstone/frankenpandas/commit/6a157919)); windows-latest lint cell ([`e516ed34`](https://github.com/Dicklesworthstone/frankenpandas/commit/e516ed34)); concurrency cancel-in-progress ([`b3dda848`](https://github.com/Dicklesworthstone/frankenpandas/commit/b3dda848)); doctests in CI ([`128eec76`](https://github.com/Dicklesworthstone/frankenpandas/commit/128eec76)); GitHub Actions version bumps ([`20c46606`](https://github.com/Dicklesworthstone/frankenpandas/commit/20c46606)); dated nightly toolchain pin ([`af943cc6`](https://github.com/Dicklesworthstone/frankenpandas/commit/af943cc6)); cargo-audit + cargo-deny gates ([`6c1b4ce7`](https://github.com/Dicklesworthstone/frankenpandas/commit/6c1b4ce7)); perf-baseline gating ([`2c998520`](https://github.com/Dicklesworthstone/frankenpandas/commit/2c998520)); release-plz workflow ([`ee5d7837`](https://github.com/Dicklesworthstone/frankenpandas/commit/ee5d7837))
- **Fuzz infrastructure (~90 commits)**: parquet IO, arrow IPC, scalar cast, Series::add, groupby_sum, join_series, column arithmetic, feather IO, common_dtype, excel IO, index alignment, shift metamorphic, abs/clip/round/between/cumulative-extrema/nlargest/sort_values/rank/diff/cum sum-prod/idxmax-idxmin/where-mask/fillna/dropna/replace/isin/duplicated/sort_index/reindex/take/set_axis/rename/truncate/drop/head-tail/isna-notna/count/sum/mean/std-var/median proptests ([`b9295ed7`](https://github.com/Dicklesworthstone/frankenpandas/commit/b9295ed7)..[`0598a632`](https://github.com/Dicklesworthstone/frankenpandas/commit/0598a632)). Later: cross-format round-trip, dataframe eval, semantic_eq, pivot_table dispatch, groupby agg dispatch, rolling window min_periods, stateful op-chain, parallel + TSan, SQL read, DataFrame constructor + merge. Hardening: libFuzzer memory/timeout bounds ([`588a0655`](https://github.com/Dicklesworthstone/frankenpandas/commit/588a0655)), structure-aware dictionaries ([`f4eed267`](https://github.com/Dicklesworthstone/frankenpandas/commit/f4eed267)), weekly corpus minimization ([`34ae9f6a`](https://github.com/Dicklesworthstone/frankenpandas/commit/34ae9f6a)), regression corpus in CI ([`0fa67a7e`](https://github.com/Dicklesworthstone/frankenpandas/commit/0fa67a7e))
- **Release / supply-chain / docs hygiene**: SECURITY.md ([`fb455f88`](https://github.com/Dicklesworthstone/frankenpandas/commit/fb455f88)); `#[non_exhaustive]` on all public error enums ([`9b6374d4`](https://github.com/Dicklesworthstone/frankenpandas/commit/9b6374d4)); `.editorconfig` + `.gitattributes` ([`74361bf9`](https://github.com/Dicklesworthstone/frankenpandas/commit/74361bf9)); `.mailmap` + `CITATION.cff` ([`4b4170d7`](https://github.com/Dicklesworthstone/frankenpandas/commit/4b4170d7)); ISSUE/PR templates + FUNDING.yml ([`41bebe5b`](https://github.com/Dicklesworthstone/frankenpandas/commit/41bebe5b), [`d4b0d9f3`](https://github.com/Dicklesworthstone/frankenpandas/commit/d4b0d9f3), [`712028e6`](https://github.com/Dicklesworthstone/frankenpandas/commit/712028e6)); workspace.package inheritance + per-crate metadata ([`85c2854f`](https://github.com/Dicklesworthstone/frankenpandas/commit/85c2854f)); docs.rs metadata + rustfmt.toml ([`bd55f5ca`](https://github.com/Dicklesworthstone/frankenpandas/commit/bd55f5ca)); AUTHORS.md + cargo-machete ([`15535a53`](https://github.com/Dicklesworthstone/frankenpandas/commit/15535a53)); per-crate README ([`87a6cace`](https://github.com/Dicklesworthstone/frankenpandas/commit/87a6cace)); committed githooks + gitleaks CI + CODEOWNERS ([`e3761f46`](https://github.com/Dicklesworthstone/frankenpandas/commit/e3761f46)); CONTRIBUTING.md + code of conduct ([`fb501bdc`](https://github.com/Dicklesworthstone/frankenpandas/commit/fb501bdc)); SSH commit-signing policy ([`ad0623ab`](https://github.com/Dicklesworthstone/frankenpandas/commit/ad0623ab))
- **Dep bumps (library-updater sweep 2026-04-22)**: asupersync 0.2→0.3.1, libfuzzer-sys 0.4.10→0.4.12, bytes 1.10.1→1.11.1, bumpalo 3.16→3.20.2, regex 1.11.1→1.12.3, serde 1.0.219→1.0.228, serde_json 1.0.140→1.0.149, thiserror 2.0.12→2.0.18, raptorq 2.0→2.0.1, proptest 1.10→1.11, tempfile 3.14→3.27, sha2 0.10.9→0.11, calamine 0.26.1→0.34, arrow + parquet 54.3→58.1, criterion 0.5.1→0.8.2

### Phase 2c: Index-name preservation + Index variants + format expansion (2026-05-02 → 2026-05-16)

The post-mega-merge cleanup. A fork-wide audit found the alignment
planner was almost-but-not-quite preserving axis names through helper
methods. Hundreds of callsites were retrofitted to use a shared helper.
The typed-Index variants gained dozens of pandas-parity methods. The IO
layer learned ~10 additional formats. Algorithmic complexity got
another optimization pass focused on accidental O(n²) loops.

#### "Preserve index name" fork-wide sweep (~76 commits)

Index/axis-name propagation through Series, DataFrame, groupby, rolling, expanding, ewm, resample, string, and datetime accessor pipelines:

- **Single helper-conversion commit retrofitted 36 transform call sites**: [`fe61bbd3`](https://github.com/Dicklesworthstone/frankenpandas/commit/fe61bbd3) (Series helper preserves index name; convert 36 transform call sites — `frankenpandas-07yrq`)
- **Rolling**: `min`/`count`/`first`/`last`/`corr`/`cov`/`rank`/`apply_rolling` ([`f41b3e44`](https://github.com/Dicklesworthstone/frankenpandas/commit/f41b3e44), [`85e99b22`](https://github.com/Dicklesworthstone/frankenpandas/commit/85e99b22), [`390836eb`](https://github.com/Dicklesworthstone/frankenpandas/commit/390836eb), [`22761a80`](https://github.com/Dicklesworthstone/frankenpandas/commit/22761a80), [`b68c7315`](https://github.com/Dicklesworthstone/frankenpandas/commit/b68c7315), [`7868a706`](https://github.com/Dicklesworthstone/frankenpandas/commit/7868a706), [`a5dcd589`](https://github.com/Dicklesworthstone/frankenpandas/commit/a5dcd589))
- **Expanding**: `apply`/`count`/`corr`/`cov` ([`1528a3e1`](https://github.com/Dicklesworthstone/frankenpandas/commit/1528a3e1), [`078db704`](https://github.com/Dicklesworthstone/frankenpandas/commit/078db704), [`07d886f9`](https://github.com/Dicklesworthstone/frankenpandas/commit/07d886f9), [`d9b26480`](https://github.com/Dicklesworthstone/frankenpandas/commit/d9b26480))
- **Ewm**: `mean`/`sum`/`cov`/`var`/`std`/`corr` ([`34257bcd`](https://github.com/Dicklesworthstone/frankenpandas/commit/34257bcd), [`6da08de9`](https://github.com/Dicklesworthstone/frankenpandas/commit/6da08de9), [`2b780959`](https://github.com/Dicklesworthstone/frankenpandas/commit/2b780959), [`55b6c3a1`](https://github.com/Dicklesworthstone/frankenpandas/commit/55b6c3a1), [`27f7ef53`](https://github.com/Dicklesworthstone/frankenpandas/commit/27f7ef53))
- **Resample**: `aggregate_scalar`/`size`/`transform`/`get_group` ([`de8d7ad5`](https://github.com/Dicklesworthstone/frankenpandas/commit/de8d7ad5), [`10bc66d1`](https://github.com/Dicklesworthstone/frankenpandas/commit/10bc66d1), [`98ab2df3`](https://github.com/Dicklesworthstone/frankenpandas/commit/98ab2df3), [`d8151cfb`](https://github.com/Dicklesworthstone/frankenpandas/commit/d8151cfb))
- **SeriesGroupBy**: `transform`/`cumcount`/`ngroup`/`apply` ([`894af1c3`](https://github.com/Dicklesworthstone/frankenpandas/commit/894af1c3), [`902ae06e`](https://github.com/Dicklesworthstone/frankenpandas/commit/902ae06e), [`4b90cb4d`](https://github.com/Dicklesworthstone/frankenpandas/commit/4b90cb4d), [`f4348c61`](https://github.com/Dicklesworthstone/frankenpandas/commit/f4348c61))
- **DataFrame methods**: `filter_rows`/`take_rows`/`stack`/`unstack`/`pivot`/`pivot_table`/`explode`/`reindex`/`truncate`/`sample`/`rename_index*`/`select_columns`/`asof` ([`01e27f6c`](https://github.com/Dicklesworthstone/frankenpandas/commit/01e27f6c), [`c2cfa2df`](https://github.com/Dicklesworthstone/frankenpandas/commit/c2cfa2df), [`51a49570`](https://github.com/Dicklesworthstone/frankenpandas/commit/51a49570), [`5858df74`](https://github.com/Dicklesworthstone/frankenpandas/commit/5858df74), [`4b2a33af`](https://github.com/Dicklesworthstone/frankenpandas/commit/4b2a33af), [`0eb7352f`](https://github.com/Dicklesworthstone/frankenpandas/commit/0eb7352f), [`a73edf66`](https://github.com/Dicklesworthstone/frankenpandas/commit/a73edf66), [`45b91414`](https://github.com/Dicklesworthstone/frankenpandas/commit/45b91414), [`16789246`](https://github.com/Dicklesworthstone/frankenpandas/commit/16789246), [`98005696`](https://github.com/Dicklesworthstone/frankenpandas/commit/98005696), [`7b003084`](https://github.com/Dicklesworthstone/frankenpandas/commit/7b003084))
- **Series methods**: `dropna`/`drop`/`head`/`tail`/`sort`/`sample`/`combine`/`explode`/`append`/`map_*`/`apply*`/`mask*`/`where_cond*`/`rank*`/`td_*`/arithmetic; **40+ callsites**
- **StringMethods** (`partition`/`rpartition`/`cat_series`) and **DatetimeAccessor** (`extract_component`/`try_extract_component`) ([`5b7a0c06`](https://github.com/Dicklesworthstone/frankenpandas/commit/5b7a0c06), [`e1a907c6`](https://github.com/Dicklesworthstone/frankenpandas/commit/e1a907c6), [`44475630`](https://github.com/Dicklesworthstone/frankenpandas/commit/44475630), [`e097d558`](https://github.com/Dicklesworthstone/frankenpandas/commit/e097d558), [`4e699761`](https://github.com/Dicklesworthstone/frankenpandas/commit/4e699761))

#### Typed Index variants — massive method-surface buildout (2026-05-09)

Single-day burst adding pandas-parity methods to `DatetimeIndex`, `TimedeltaIndex`, `PeriodIndex`, `RangeIndex`, `CategoricalIndex` via per-variant typed forwarders. Method coverage now includes:

- Time-of-day accessors (`hour`/`minute`/`second`/`microsecond`/`nanosecond`), day-of-X/quarter/leap, month/quarter/year start/end boundaries
- `month_name`/`day_name`/`normalize`, `asi8`/`strftime`
- `argmax`/`argmin`/`argsort`/`unique`/`factorize`/`value_counts`/`duplicated`/`drop_duplicates`/`dropna`
- `take`/`repeat`/`isin`; `append`/`delete`/`insert`; set ops (`intersection`/`union`/`difference`/`symmetric_difference`)
- `searchsorted`, `where`/`putmask`, `tz`/`freq`/`inferred_freq`
- `to_pydatetime`/`to_pytimedelta`/`to_julian_date`, `tz_localize`/`tz_convert`
- `slice_locs`/`slice_indexer`/`get_slice_bound`, `unit`/`resolution`/`as_unit`
- `get_loc`/`get_indexer`/`get_indexer_non_unique`/`get_indexer_for`
- `mean`/`median`/`std`/`var`/`sum`, `shift`, `fillna`/`isnull`/`notnull`
- **Period-specific**: `is_full`, `from_ordinals`, `from_fields`, `to_flat_index`
- **DatetimeIndex** → `to_period` ([`741ef1be`](https://github.com/Dicklesworthstone/frankenpandas/commit/741ef1be)); PeriodIndex `asfreq` ([`b3047da0`](https://github.com/Dicklesworthstone/frankenpandas/commit/b3047da0))

Representative commits: [`190ae663`](https://github.com/Dicklesworthstone/frankenpandas/commit/190ae663), [`6b44c46d`](https://github.com/Dicklesworthstone/frankenpandas/commit/6b44c46d), [`69901939`](https://github.com/Dicklesworthstone/frankenpandas/commit/69901939), [`a4207b31`](https://github.com/Dicklesworthstone/frankenpandas/commit/a4207b31), [`a42cd4ea`](https://github.com/Dicklesworthstone/frankenpandas/commit/a42cd4ea), [`0fe22520`](https://github.com/Dicklesworthstone/frankenpandas/commit/0fe22520), [`61bf5192`](https://github.com/Dicklesworthstone/frankenpandas/commit/61bf5192), [`ee08d8ac`](https://github.com/Dicklesworthstone/frankenpandas/commit/ee08d8ac)

#### MultiIndex parity epic (frankenpandas-d89fe + 18 sub-slices)

- `get_indexer`/`get_indexer_for` ([`2c3db986`](https://github.com/Dicklesworthstone/frankenpandas/commit/2c3db986)); `get_slice_bound`/`slice_indexer` ([`df75b90b`](https://github.com/Dicklesworthstone/frankenpandas/commit/df75b90b))
- `groupby`/`join` ([`4cf2c451`](https://github.com/Dicklesworthstone/frankenpandas/commit/4cf2c451)); `reindex` ([`85db047a`](https://github.com/Dicklesworthstone/frankenpandas/commit/85db047a)); `rename` ([`e1eb5aec`](https://github.com/Dicklesworthstone/frankenpandas/commit/e1eb5aec)); `searchsorted` ([`1f34c499`](https://github.com/Dicklesworthstone/frankenpandas/commit/1f34c499))
- `all`/`any`/missing-mask/`shift` error parity ([`59257d19`](https://github.com/Dicklesworthstone/frankenpandas/commit/59257d19), [`5f373b5e`](https://github.com/Dicklesworthstone/frankenpandas/commit/5f373b5e), [`ddd42030`](https://github.com/Dicklesworthstone/frankenpandas/commit/ddd42030))
- `get_locs` list-label parity ([`8ebb7e96`](https://github.com/Dicklesworthstone/frankenpandas/commit/8ebb7e96)); `truncate` ([`f435e6c7`](https://github.com/Dicklesworthstone/frankenpandas/commit/f435e6c7)); `from_frame` ([`f59e68d8`](https://github.com/Dicklesworthstone/frankenpandas/commit/f59e68d8))

#### SeriesGroupBy / DataFrameGroupBy parity build (frankenpandas-nt65g + 12 slices)

`transform`/`filter`/`pipe`, `take`, `sample`, `quantile`/`sem`/`skew`, missing-value handling, ranked selection, `value_counts`, `unique`, `ohlc`, monotonic/null parity, `first`/`last` null, string `min`/`max`, `apply` callbacks, rolling/expanding/ewm/resample reductions on SeriesGroupBy, `describe`, pairwise `corr`/`cov`, grouper introspection. Representative: [`1b89cb12`](https://github.com/Dicklesworthstone/frankenpandas/commit/1b89cb12), [`8431c3cc`](https://github.com/Dicklesworthstone/frankenpandas/commit/8431c3cc), [`d5005b6b`](https://github.com/Dicklesworthstone/frankenpandas/commit/d5005b6b), [`96a4dfce`](https://github.com/Dicklesworthstone/frankenpandas/commit/96a4dfce), [`eb51da63`](https://github.com/Dicklesworthstone/frankenpandas/commit/eb51da63), [`d2bff302`](https://github.com/Dicklesworthstone/frankenpandas/commit/d2bff302), [`24facdfe`](https://github.com/Dicklesworthstone/frankenpandas/commit/24facdfe)

#### Algorithmic complexity sweep — O(n²) → O(n) (15+ commits)

Hot-path rewrites using `HashMap`/`HashSet`/`IsinIndex`:

- **`value_counts`** ([`b6f4943c`](https://github.com/Dicklesworthstone/frankenpandas/commit/b6f4943c)), `unique`/`nunique_with_dropna` ([`60856d44`](https://github.com/Dicklesworthstone/frankenpandas/commit/60856d44)), `duplicated`/`drop_duplicates` ([`e58fefea`](https://github.com/Dicklesworthstone/frankenpandas/commit/e58fefea))
- `map`/`replace`/`map_with_default` ([`704f121d`](https://github.com/Dicklesworthstone/frankenpandas/commit/704f121d)); `isin` for Series and DataFrame ([`5fab8d79`](https://github.com/Dicklesworthstone/frankenpandas/commit/5fab8d79))
- `cut()` O(1)-bucket ([`51eb3636`](https://github.com/Dicklesworthstone/frankenpandas/commit/51eb3636)); `qcut()` binary search ([`70c2ae8e`](https://github.com/Dicklesworthstone/frankenpandas/commit/70c2ae8e))
- `mode_with_dropna` ([`9ec24cfc`](https://github.com/Dicklesworthstone/frankenpandas/commit/9ec24cfc)); `mode_values` cross-dtype ([`20a22970`](https://github.com/Dicklesworthstone/frankenpandas/commit/20a22970))
- **DataFrame** `nunique` ([`b22e5eae`](https://github.com/Dicklesworthstone/frankenpandas/commit/b22e5eae)), `value_counts` ([`c3442b46`](https://github.com/Dicklesworthstone/frankenpandas/commit/c3442b46)), `append` column union ([`667a2a65`](https://github.com/Dicklesworthstone/frankenpandas/commit/667a2a65)); groupby `value_counts` ([`43aaa37a`](https://github.com/Dicklesworthstone/frankenpandas/commit/43aaa37a))
- **Series** `drop` ([`929e155a`](https://github.com/Dicklesworthstone/frankenpandas/commit/929e155a)), `unstack` ([`8504bac5`](https://github.com/Dicklesworthstone/frankenpandas/commit/8504bac5)), `str.get_dummies` ([`fb3dd650`](https://github.com/Dicklesworthstone/frankenpandas/commit/fb3dd650)), `factorize` ([`44f14e70`](https://github.com/Dicklesworthstone/frankenpandas/commit/44f14e70))
- **DataFrame** `get_dummies` ([`4319cb81`](https://github.com/Dicklesworthstone/frankenpandas/commit/4319cb81)); `fp-columnar` `factorize_with_options` ([`a8af1ed5`](https://github.com/Dicklesworthstone/frankenpandas/commit/a8af1ed5))
- CSV NA HashSet ([`28bc8ff2`](https://github.com/Dicklesworthstone/frankenpandas/commit/28bc8ff2), [`824b9691`](https://github.com/Dicklesworthstone/frankenpandas/commit/824b9691)); Excel sheet HashSet ([`af8164c1`](https://github.com/Dicklesworthstone/frankenpandas/commit/af8164c1))

#### IO format surface expansion — phase 2 (2026-05-08)

Single-day burst adding 10+ new reader/writer surfaces:

- **HTML writer** ([`738620d1`](https://github.com/Dicklesworthstone/frankenpandas/commit/738620d1)), **HTML reader** ([`52c6e304`](https://github.com/Dicklesworthstone/frankenpandas/commit/52c6e304)); **`to_xml` alias** ([`bc0cfc13`](https://github.com/Dicklesworthstone/frankenpandas/commit/bc0cfc13))
- **XML writer** ([`d14eb7f9`](https://github.com/Dicklesworthstone/frankenpandas/commit/d14eb7f9)), **XML reader** ([`49cf2073`](https://github.com/Dicklesworthstone/frankenpandas/commit/49cf2073))
- **Pickle roundtrip** ([`23e637ff`](https://github.com/Dicklesworthstone/frankenpandas/commit/23e637ff)); **Stata roundtrip** ([`87a2812e`](https://github.com/Dicklesworthstone/frankenpandas/commit/87a2812e))
- **ORC roundtrip** ([`443db4a0`](https://github.com/Dicklesworthstone/frankenpandas/commit/443db4a0)); **HDF5 snapshot** ([`12bef71b`](https://github.com/Dicklesworthstone/frankenpandas/commit/12bef71b)) — HDF5 later gated optional ([`ec0b828e`](https://github.com/Dicklesworthstone/frankenpandas/commit/ec0b828e))
- **Markdown / LaTeX file writers** ([`19fb3b6b`](https://github.com/Dicklesworthstone/frankenpandas/commit/19fb3b6b)); pure-string Markdown/LaTeX ([`8e675fe9`](https://github.com/Dicklesworthstone/frankenpandas/commit/8e675fe9))
- **`read_table` TSV** ([`4c308492`](https://github.com/Dicklesworthstone/frankenpandas/commit/4c308492)); **`read_fwf`** ([`94f3a504`](https://github.com/Dicklesworthstone/frankenpandas/commit/94f3a504)) + colspec inference ([`486c0768`](https://github.com/Dicklesworthstone/frankenpandas/commit/486c0768))
- Deferred-status hooks: `to_clipboard`/`to_gbq`/etc. ([`769f03b4`](https://github.com/Dicklesworthstone/frankenpandas/commit/769f03b4))

#### Datetime / Timedelta accessor + component buildout

- **`DatetimeAccessor` errors on non-datetimelike Series** instead of silent NaN ([`9c72f8c7`](https://github.com/Dicklesworthstone/frankenpandas/commit/9c72f8c7))
- `td_*` family errors on non-Timedelta operands ([`b2d45778`](https://github.com/Dicklesworthstone/frankenpandas/commit/b2d45778))
- **Datetime time indexers** ([`b5fb9940`](https://github.com/Dicklesworthstone/frankenpandas/commit/b5fb9940)); **`timetz` accessor** ([`4b95aea6`](https://github.com/Dicklesworthstone/frankenpandas/commit/4b95aea6)); **`isocalendar` accessor** ([`a1641863`](https://github.com/Dicklesworthstone/frankenpandas/commit/a1641863))
- **Timedelta `components` accessor** ([`588948b0`](https://github.com/Dicklesworthstone/frankenpandas/commit/588948b0))
- **Period → Timestamp helpers** ([`ea8587dd`](https://github.com/Dicklesworthstone/frankenpandas/commit/ea8587dd)); **DataFrame `to_timestamp`** ([`acce487d`](https://github.com/Dicklesworthstone/frankenpandas/commit/acce487d)); **DataFrame `to_period`** ([`6fce739d`](https://github.com/Dicklesworthstone/frankenpandas/commit/6fce739d))
- ISO week/`weekofyear` ([`7ccd47be`](https://github.com/Dicklesworthstone/frankenpandas/commit/7ccd47be))

#### Timedelta64 fast paths across reduction family (2026-05-13)

Added Timedelta64 fast paths or fixes to: `nanmin`/`nanmax` ([`1a5685b0`](https://github.com/Dicklesworthstone/frankenpandas/commit/1a5685b0)), `nansum`/`nanmean` ([`5231c6ff`](https://github.com/Dicklesworthstone/frankenpandas/commit/5231c6ff)), `nancumsum`/`nancummin`/`nancummax` ([`4d52a33c`](https://github.com/Dicklesworthstone/frankenpandas/commit/4d52a33c)), `nanmedian`/`nanvar`/`nanstd`/`nansem` ([`42d1d211`](https://github.com/Dicklesworthstone/frankenpandas/commit/42d1d211)), `nanquantile` ([`11ad77b0`](https://github.com/Dicklesworthstone/frankenpandas/commit/11ad77b0)), `nanargmax`/`nanargmin`/`nanptp`/`nanprod` ([`7140bc35`](https://github.com/Dicklesworthstone/frankenpandas/commit/7140bc35)), Column `pct_change` ([`b8a81e1b`](https://github.com/Dicklesworthstone/frankenpandas/commit/b8a81e1b)), Column `diff` ([`360f2ad1`](https://github.com/Dicklesworthstone/frankenpandas/commit/360f2ad1)), Series/DataFrame `diff` family ([`0e48b304`](https://github.com/Dicklesworthstone/frankenpandas/commit/0e48b304)), Series `pct_change` ([`494fef01`](https://github.com/Dicklesworthstone/frankenpandas/commit/494fef01)), GroupBy `pct_change` ([`218a6d73`](https://github.com/Dicklesworthstone/frankenpandas/commit/218a6d73)), SeriesGroupBy `sum`/`mean` ([`fa376ad0`](https://github.com/Dicklesworthstone/frankenpandas/commit/fa376ad0)) and `min`/`max` ([`2505393b`](https://github.com/Dicklesworthstone/frankenpandas/commit/2505393b)). Closes a wave of silent-stub bugs on Timedelta64.

#### DataFrame plotting / sparse / style / xarray / duplicate-label capabilities

- Deferred plotting stubs ([`ce39f75e`](https://github.com/Dicklesworthstone/frankenpandas/commit/ce39f75e)) → returned **plotting specs** (`PlotSpec`/`BoxPlotSpec`) ([`889f0517`](https://github.com/Dicklesworthstone/frankenpandas/commit/889f0517))
- **`DataFrame::to_xarray`** snapshot ([`497ce7c0`](https://github.com/Dicklesworthstone/frankenpandas/commit/497ce7c0))
- **Sparse metrics** ([`c7992d94`](https://github.com/Dicklesworthstone/frankenpandas/commit/c7992d94)); **persisted duplicate-label flags** ([`8e57aabb`](https://github.com/Dicklesworthstone/frankenpandas/commit/8e57aabb))
- **Styled DataFrame HTML rendering** (`StyledDataFrame` + `DataFrame::style()`) ([`1d26c1c1`](https://github.com/Dicklesworthstone/frankenpandas/commit/1d26c1c1))
- **Series list accessor** ([`dd47e260`](https://github.com/Dicklesworthstone/frankenpandas/commit/dd47e260)); **Series struct accessor** ([`d9b6c8f3`](https://github.com/Dicklesworthstone/frankenpandas/commit/d9b6c8f3))

#### Major fixes in this window

**SQL reader correctness bundle (frankenpandas-fd90.*, 2026-05-02 / 03)**

Honor SQL table columns in chunked reads ([`1d71f6ec`](https://github.com/Dicklesworthstone/frankenpandas/commit/1d71f6ec)), expose SQL inspector backend caps ([`7d416d1f`](https://github.com/Dicklesworthstone/frankenpandas/commit/7d416d1f)), include `index_col` in table projections ([`48a2f0bb`](https://github.com/Dicklesworthstone/frankenpandas/commit/48a2f0bb)), reject `options.index_col` on chunked entrypoints ([`5eb339a3`](https://github.com/Dicklesworthstone/frankenpandas/commit/5eb339a3)), auto-project `index_col` in column-list readers ([`ad56d0df`](https://github.com/Dicklesworthstone/frankenpandas/commit/ad56d0df)), bound SQL chunk reads ([`b75209ce`](https://github.com/Dicklesworthstone/frankenpandas/commit/b75209ce)), validate SQL builder identifier lengths ([`49bfd249`](https://github.com/Dicklesworthstone/frankenpandas/commit/49bfd249)), preserve SQL dtype hints for empty reads ([`1cdd7377`](https://github.com/Dicklesworthstone/frankenpandas/commit/1cdd7377)), reject unsupported SQL schema reads ([`c1b4e3b2`](https://github.com/Dicklesworthstone/frankenpandas/commit/c1b4e3b2)), rollback rusqlite panic transactions ([`4b224555`](https://github.com/Dicklesworthstone/frankenpandas/commit/4b224555)).

**Pandas-faithful validation hardening**

`ewm` validates inputs eagerly ([`2bf5da29`](https://github.com/Dicklesworthstone/frankenpandas/commit/2bf5da29)); rolling `min_periods <= window` validation ([`9735a0b2`](https://github.com/Dicklesworthstone/frankenpandas/commit/9735a0b2)); resample `freq` string validation ([`85541b58`](https://github.com/Dicklesworthstone/frankenpandas/commit/85541b58)); `Series::sample` frac validation ([`f5a6c62f`](https://github.com/Dicklesworthstone/frankenpandas/commit/f5a6c62f)) + DataFrame mirror ([`2e16683a`](https://github.com/Dicklesworthstone/frankenpandas/commit/2e16683a)) + groupby ([`4a1dd8de`](https://github.com/Dicklesworthstone/frankenpandas/commit/4a1dd8de)); `str.pad` side validation ([`3da82b64`](https://github.com/Dicklesworthstone/frankenpandas/commit/3da82b64)); `sort_values_na` `na_position` ([`10706b1a`](https://github.com/Dicklesworthstone/frankenpandas/commit/10706b1a)); `astype_safe` `errors` ([`2c6b2d62`](https://github.com/Dicklesworthstone/frankenpandas/commit/2c6b2d62)); `Series::drop` rejects missing labels ([`a4b2ffc8`](https://github.com/Dicklesworthstone/frankenpandas/commit/a4b2ffc8)); `DataFrame::drop(axis=0)` rejects missing rows ([`eea0a64f`](https://github.com/Dicklesworthstone/frankenpandas/commit/eea0a64f)); `DataFrame::insert` rejects out-of-bounds loc ([`e71c5063`](https://github.com/Dicklesworthstone/frankenpandas/commit/e71c5063)); reject `between_time`/`at_time` on non-DatetimeIndex ([`ecc60ef6`](https://github.com/Dicklesworthstone/frankenpandas/commit/ecc60ef6)); reject `Timedelta::parse` overflow ([`a960003a`](https://github.com/Dicklesworthstone/frankenpandas/commit/a960003a)); reject invalid `filter_labels` ([`1182ea49`](https://github.com/Dicklesworthstone/frankenpandas/commit/1182ea49)); reject empty `groupby` by-list ([`9a93748e`](https://github.com/Dicklesworthstone/frankenpandas/commit/9a93748e)); reject negative/NaN `sample_weights` ([`8f9026d1`](https://github.com/Dicklesworthstone/frankenpandas/commit/8f9026d1)); `read_jsonl_str` row cap ([`f1cc613d`](https://github.com/Dicklesworthstone/frankenpandas/commit/f1cc613d)); `date_range` overflow rejection ([`378a0663`](https://github.com/Dicklesworthstone/frankenpandas/commit/378a0663)) and over-specified ranges ([`829dc0dc`](https://github.com/Dicklesworthstone/frankenpandas/commit/829dc0dc)).

**Utf8 semantic-compare sweep**

`cummin`/`cummax` Utf8 lexicographic ([`bf0935a9`](https://github.com/Dicklesworthstone/frankenpandas/commit/bf0935a9)); `min`/`max` Utf8 ([`ef3f6165`](https://github.com/Dicklesworthstone/frankenpandas/commit/ef3f6165)); `idxmin`/`idxmax` Utf8 ([`1a8567ba`](https://github.com/Dicklesworthstone/frankenpandas/commit/1a8567ba)); `nlargest`/`nsmallest` Utf8 ([`6eb918de`](https://github.com/Dicklesworthstone/frankenpandas/commit/6eb918de)); `rank` Utf8 ([`455a3d5a`](https://github.com/Dicklesworthstone/frankenpandas/commit/455a3d5a), [`d8b8301a`](https://github.com/Dicklesworthstone/frankenpandas/commit/d8b8301a)); `cumsum` Utf8 concat ([`a41619dc`](https://github.com/Dicklesworthstone/frankenpandas/commit/a41619dc)); `sum` Utf8 concat ([`3e18f7e7`](https://github.com/Dicklesworthstone/frankenpandas/commit/3e18f7e7)); `clip_with_series` Utf8 ([`d6425a99`](https://github.com/Dicklesworthstone/frankenpandas/commit/d6425a99)); `GroupBy::idxmin/idxmax` Utf8 ([`d61545fa`](https://github.com/Dicklesworthstone/frankenpandas/commit/d61545fa)); `is_monotonic_increasing` Utf8/Bool ([`559a9f2c`](https://github.com/Dicklesworthstone/frankenpandas/commit/559a9f2c)).

**Numeric dtype preservation**

`Series::clip` Int64 preservation ([`85a7a45e`](https://github.com/Dicklesworthstone/frankenpandas/commit/85a7a45e)); `min`/`max` Int64/Bool preservation ([`f76bc051`](https://github.com/Dicklesworthstone/frankenpandas/commit/f76bc051)); `sum`/`prod` Int64/Bool with numpy-wrap ([`bf860c7a`](https://github.com/Dicklesworthstone/frankenpandas/commit/bf860c7a)); `DataFrame.cumsum`/`cumprod`/`diff`/`shift` on Bool columns ([`f88e97e4`](https://github.com/Dicklesworthstone/frankenpandas/commit/f88e97e4)); `skew`/`kurtosis` NaN-on-too-few ([`37980128`](https://github.com/Dicklesworthstone/frankenpandas/commit/37980128)); `sem` NaN on undersized ([`ed74fef8`](https://github.com/Dicklesworthstone/frankenpandas/commit/ed74fef8)); `is_unique` single-null fix ([`ea0407a9`](https://github.com/Dicklesworthstone/frankenpandas/commit/ea0407a9)); promote int `FloorDiv`/`Mod` when right has zero ([`cb412d42`](https://github.com/Dicklesworthstone/frankenpandas/commit/cb412d42)); coerce_scalar Float64→Int64 range check ([`ef87da6f`](https://github.com/Dicklesworthstone/frankenpandas/commit/ef87da6f)); stop promoting Int64→Float64 in groupby `first`/`last` ([`aa3e464c`](https://github.com/Dicklesworthstone/frankenpandas/commit/aa3e464c)); NaN clip bounds treated as no-clipping ([`ca783d52`](https://github.com/Dicklesworthstone/frankenpandas/commit/ca783d52)).

**String method char-position fix**

`str.find`/`rfind`/`index_of`/`rindex_of` returned BYTE positions instead of CHAR ([`9de63cdf`](https://github.com/Dicklesworthstone/frankenpandas/commit/9de63cdf)) + matching live-oracle and proptest coverage. Plus string-split expansion-limit overflow guard ([`26142ac7`](https://github.com/Dicklesworthstone/frankenpandas/commit/26142ac7)).

**Rolling / window panic hardening**

`ewm.sum` Utf8 panic guard ([`8afc13be`](https://github.com/Dicklesworthstone/frankenpandas/commit/8afc13be)); `Rolling::first`/`last` all-missing window guard ([`81869934`](https://github.com/Dicklesworthstone/frankenpandas/commit/81869934)); `Rolling::max`/`prod` empty-window returns NaN ([`44b012c7`](https://github.com/Dicklesworthstone/frankenpandas/commit/44b012c7)); pandas-canonical `.0` suffix for whole-number Float64 in `to_csv` ([`673dfc07`](https://github.com/Dicklesworthstone/frankenpandas/commit/673dfc07)).

**Asupersync recovery & evidence**

Enforce recovery deadlines ([`c42920b1`](https://github.com/Dicklesworthstone/frankenpandas/commit/c42920b1)); fail-closed on unchecked asupersync evidence ([`691f0a53`](https://github.com/Dicklesworthstone/frankenpandas/commit/691f0a53)); replace recovery `StubCodec` ([`f030cd6c`](https://github.com/Dicklesworthstone/frankenpandas/commit/f030cd6c)); bound `recover_once` retry loop under buggy policy ([`454b3b60`](https://github.com/Dicklesworthstone/frankenpandas/commit/454b3b60)); reject zero repair-symbol decodes ([`138a54ea`](https://github.com/Dicklesworthstone/frankenpandas/commit/138a54ea)).

#### Infrastructure / tooling

- **Beads hygiene infrastructure**: bead-hygiene verifier ([`b0c4ad59`](https://github.com/Dicklesworthstone/frankenpandas/commit/b0c4ad59)); API coverage drift gate ([`49d840e1`](https://github.com/Dicklesworthstone/frankenpandas/commit/49d840e1)); cross-db scipy contamination sweep ([`a1929a95`](https://github.com/Dicklesworthstone/frankenpandas/commit/a1929a95)); JSONL race resolution + close-wins collapse ([`a5f69167`](https://github.com/Dicklesworthstone/frankenpandas/commit/a5f69167), [`baaf3aa4`](https://github.com/Dicklesworthstone/frankenpandas/commit/baaf3aa4)); DB repair + 31-bead flush ([`531b2683`](https://github.com/Dicklesworthstone/frankenpandas/commit/531b2683)); no-db JSONL malformed-row repair ([`c4eacf4d`](https://github.com/Dicklesworthstone/frankenpandas/commit/c4eacf4d))
- **Phase2c attestation artifacts** refreshed multiple times; 7 new fp-conformance failure-surface JSONLs captured
- **CI**: cargo-audit ignores RUSTSEC-2024-0436 paste-unmaintained ([`1e5121e5`](https://github.com/Dicklesworthstone/frankenpandas/commit/1e5121e5)); SPDX MIT license workspace-wide with asupersync MIT-with-rider clarification ([`1e5be685`](https://github.com/Dicklesworthstone/frankenpandas/commit/1e5be685)); CI requires system pandas live oracle fallback ([`ec36ff1d`](https://github.com/Dicklesworthstone/frankenpandas/commit/ec36ff1d))
- **Test infra**: pandas error compatibility catalog ([`240cdb7e`](https://github.com/Dicklesworthstone/frankenpandas/commit/240cdb7e)); generated fixture pilot sidecars ([`69130a3a`](https://github.com/Dicklesworthstone/frankenpandas/commit/69130a3a)); AACE alignment witness ledger ([`0b9005bc`](https://github.com/Dicklesworthstone/frankenpandas/commit/0b9005bc)); high-RAM perf baseline telemetry ([`a3c7a99f`](https://github.com/Dicklesworthstone/frankenpandas/commit/a3c7a99f)); metamorphic test families for null/NaN/join/reshape/value_counts; right-join perf-budget coverage ([`d13568b9`](https://github.com/Dicklesworthstone/frankenpandas/commit/d13568b9)); `fuzz_column_arith` crash regression artifact ([`48941941`](https://github.com/Dicklesworthstone/frankenpandas/commit/48941941))

---

## Phase 3: Measured kernels, Python bindings, published releases (2026-05-17 → 2026-08-19)

5,077 non-merge commits after the 2026-05-16 snapshot; 1,910 beads closed in the window. This is
the era that **actually published GitHub Releases** (`fp-python-v0.1.0` 2026-05-25, workspace
`v0.1.1` 2026-06-21, `v0.1.2` 2026-06-22, `frankenpandas-v0.2.0` 2026-07-28) and replaced the
"Rust-only facade" story with a real `fp-python` surface plus a measured-vs-pandas kernel campaign.

### Repo-janitor docs reorganization (2026-08-18 .. 2026-08-19)

Root planning/parity docs (`FEATURE_PARITY.md`, `EXHAUSTIVE_LEGACY_ANALYSIS.md`,
`PLAN_TO_PORT_PANDAS_TO_RUST.md`, and siblings) now live under [`docs/planning/`](docs/planning/).
Skill-loop scratch and already-gitignored beads recovery snapshots were untracked.

- Untrack skill-loop scratch; move root planning docs into `docs/planning/`
  ([87c6892](https://github.com/Dicklesworthstone/frankenpandas/commit/87c6892f35b3579c71fd8df91a1218961fbad67b))
- Untrack beads recovery snapshots already gitignored
  ([42b85ea](https://github.com/Dicklesworthstone/frankenpandas/commit/42b85ea06bedc3491857f0b1f9f9188c061b0222))
- Move remaining root planning/parity docs into `docs/planning/`
  ([5420971](https://github.com/Dicklesworthstone/frankenpandas/commit/5420971c9ec00c11b09e351b000476fcaa457eab))

### Phase 3d: Oracle/parity tail and domain-fused math (2026-08-01 → 2026-08-19)

August (~1,275 non-merge commits; 464 beads closed) is a correctness month on the live pandas
oracle — constructor null/bool/utf8 coercion, datetime `unit=` / `.dt` timezone drop,
`groupby().rolling()` row order — plus measured f64 `mod`/`floordiv` kernels that recover
6.83× / 10.41× bit-identical to `npy_divmod` (`br-frankenpandas-7yiuz`).

- `mod` f64 remainder recovers 6.83× with a fused multiply-add, bit-identical to `npy_divmod`
  ([e8897b1](https://github.com/Dicklesworthstone/frankenpandas/commit/e8897b18246d03162fa3d043274b08357767881f))
- Guarded fast path restores `floordiv` f64 — 10.41× over the `fmod` path
  ([e3ba5d4](https://github.com/Dicklesworthstone/frankenpandas/commit/e3ba5d4721ef942ebf946701d659171462a62411))

### Phase 3c: Radix sorts, affine searchsorted, groupby rolling scatter, 0.2.0 (2026-07-01 → 2026-07-28)

July (~794 non-merge commits; 182 beads closed) ships nullable radix sorts (Int64 / Float64 /
Datetime64 / Timedelta64 / Utf8, na-last), affine `searchsorted` at 33.18×, chunked-parallel
`to_csv` at 5.13×, and typed-output scatter for DataFrameGroupBy rolling. Workspace version
bumps 0.1.2 → 0.2.0 on 2026-07-26; GitHub Release [`frankenpandas-v0.2.0`](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/frankenpandas-v0.2.0) publishes 2026-07-28.

- Chunked-parallel `to_csv` typed serialization (5.13×)
  ([fb13f22](https://github.com/Dicklesworthstone/frankenpandas/commit/fb13f224c03d69019ff1dc011d3a201ad9374408))
- Close affine `searchsorted` (33.18×)
  ([a1c7c03](https://github.com/Dicklesworthstone/frankenpandas/commit/a1c7c03bb1d2bc278633ceb02a735a33e0eeec0c))
- Nullable Datetime64/Timedelta64 sort/argsort — na-last radix (4.29× dt64 / 3.99× td64)
  ([c04108f](https://github.com/Dicklesworthstone/frankenpandas/commit/c04108fd865975e358917edfd1782d1702f9edb7))
- Nullable/NaN Float64 sort/argsort — na-last radix (3.85× null / 3.77× NaN)
  ([f3ad443](https://github.com/Dicklesworthstone/frankenpandas/commit/f3ad443170ec60492e08cc13cc94bc48d9c59d06))
- Nullable Int64 sort/argsort — na-last radix (2.796× asc / 2.463× desc)
  ([5ad52e0](https://github.com/Dicklesworthstone/frankenpandas/commit/5ad52e0c6056185dae5f3a107b2884166d441ecb))
- Typed nullable-Utf8 `sort_values` (na-last radix) — 3.3×
  ([730fe54](https://github.com/Dicklesworthstone/frankenpandas/commit/730fe54a05febab4077cef247f91e0c54adf203f))
- DataFrameGroupBy rolling typed-output f64+validity scatter (1.63×) (`br-frankenpandas-u5900`)
  ([8cc5158](https://github.com/Dicklesworthstone/frankenpandas/commit/8cc51582f501e1c9621d70f3ee23793c00a44c6d))
- Workspace 0.1.2 → 0.2.0
  ([07ec72b](https://github.com/Dicklesworthstone/frankenpandas/commit/07ec72b34a1623d44be110d46f30bb8b9703f2e0))

### Phase 3b: `fp-python` surface, join/frame kernels that beat pandas, v0.1.1 / v0.1.2 (2026-06-01 → 2026-06-30)

June is the largest month (~2,382 non-merge commits; 830 beads closed). `fp-python` grows a real
DataFrame/Series surface (`map`/`replace`/`astype`, `merge`, `isna`/`clip`, `to_csv`,
`read_json`/`parquet`). String-key inner join stops cloning `String`s (1.30×). Expanding
skew/kurt fuse + `sqrt` instead of `powf` flips a 0.43× loss to 1.19× @1M so fp **beats pandas**
on that row. Workspace GitHub Releases: [`v0.1.1`](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/v0.1.1) (2026-06-21), [`v0.1.2`](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/v0.1.2) (2026-06-22).

- Byte-span build+probe for string-key inner join — 1.30×, zero `String` clones (`br-frankenpandas-i388q`)
  ([9cb0fa2](https://github.com/Dicklesworthstone/frankenpandas/commit/9cb0fa233d1b807a8a0b300d310236b4c070cdb6))
- `fp-python`: `map`/`replace`/`astype`
  ([76a1249](https://github.com/Dicklesworthstone/frankenpandas/commit/76a1249dced5bbf217c86d68bc47ec2cbc234c67))
- `fp-python`: `DataFrame.merge` (joins via `fp-join`)
  ([a6a2770](https://github.com/Dicklesworthstone/frankenpandas/commit/a6a2770c369a547db04975adc687dce34d221dce))
- `fp-python`: expose `read_json`/`read_jsonl`/`read_parquet`
  ([694b5db](https://github.com/Dicklesworthstone/frankenpandas/commit/694b5db47ae45806aa8b54b781d3f2be46cb0722))
- Expanding skew `s2^1.5` via `sqrt` not `powf` — 0.43×→1.19× @1M (fp now beats pandas)
  ([cee21d4](https://github.com/Dicklesworthstone/frankenpandas/commit/cee21d4d66fdc2969a5c4e70f84442cd26a11cf2))
- Expanding skew/kurt fuse — drop O(n log n) `BTreeMap` + typed output (5.9× fp-side, bit-identical)
  ([8483fe3](https://github.com/Dicklesworthstone/frankenpandas/commit/8483fe39ffc68da0383c4ecae96d77a3ec63e09b))

### Phase 3a: Datetime/rolling/resample parity and first `fp-python` Release (2026-05-17 → 2026-05-31)

Late May (~750 non-merge commits after 2026-05-16; 434 beads closed in the month) closes
datetime/rolling/ewm/`dt.total_seconds` parity bugs and publishes the first
[`fp-python-v0.1.0`](https://github.com/Dicklesworthstone/frankenpandas/releases/tag/fp-python-v0.1.0)
GitHub Release on 2026-05-25.

- Resolve P1 filter regression + rolling/ewm `min_periods` bugs
  ([cd78282](https://github.com/Dicklesworthstone/frankenpandas/commit/cd78282de8c985b7d906f910f8c081e2fbe7d00b))
- `dt.round`, rolling count, Excel round-trip, `dt.total_seconds` parity
  ([4e44c60](https://github.com/Dicklesworthstone/frankenpandas/commit/4e44c6090ca2792d90594906942a56016e7f5f4a))

---

## Commit Statistics (snapshot 2026-05-16; not re-audited on 2026-08-19)

| Metric | Value |
|--------|-------|
| Total commits | **2,796** |
| Date range | 2026-02-13 → 2026-05-16 |
| Tags / releases | None *as of this 2026-05-16 snapshot* (0.1.0 publish was still tracked by `br-frankenpandas-4clx`). Subsequent GitHub Releases: see the Version timeline at the top of this file. |
| Workspace crates | **12** |
| Rust lines (all `.rs` in `crates/`, incl. test dirs) | **269,398** |
| Rust lines under `src/` (all `.rs` incl. inline `#[cfg(test)]`) | **250,330** |
| `#[test]` / `#[tokio::test]` markers in `src/` | **5,173** |
| Public `fn` / `async fn` (top-level, `src/`) | 347 (counts free functions; total public-method surface is larger) |
| Conformance fixture JSON files (all) | **1,265** |
| Conformance packet JSON files (`fixtures/packets/`) | **1,252** |
| Documented divergences (`DISCREPANCIES.md`) | 14 (2 RESOLVED — DISC-005 + DISC-013; 12 active / ACCEPTED / INVESTIGATING / WILL-FIX) |
| Beads tracked | 1,988 lines (1,986 closed; 2 open) |
| IO formats supported | 14+ (CSV, TSV, FWF, JSON, JSONL, Parquet, Excel, Feather, IPC stream, SQL, HTML, XML, LaTeX, Markdown, Pickle, Stata, ORC, HDF5; plus deferred clipboard/gbq/sas) |
| License | MIT + non-MIT AI-assistant rider |

*(Numbers come from: `git log --oneline | wc -l` for commits; `find crates -name '*.rs' | xargs wc -l` for LOC; `grep -rE '^pub (fn|async fn)' crates/*/src/ | wc -l` for free-fn count; `grep -rE '#\[(test|tokio::test)\]' crates/*/src/ | wc -l` for test markers; `find crates/fp-conformance/fixtures -name '*.json' | wc -l` and `ls crates/fp-conformance/fixtures/packets | wc -l` for fixture/packet totals; `wc -l .beads/issues.jsonl` and `jq -r .status .beads/issues.jsonl | sort | uniq -c` for bead state.)*

---

## Open Workstreams (snapshot as of 2026-05-16)

This block is the 2026-05-16 audit. It is **not** the 2026-08-19 tracker state
(3,901 closed / 63 open / 7 in_progress as of the Phase 3 refresh).

Only **2 open beads** remain in the tracker *as of 2026-05-16*:

- **`br-frankenpandas-ctmet`** — 25 conformance packets expect Float64 where code returns Int64/Bool (`cumsum`/`cumprod`/`describe`/`round` dtype drift).
- **`br-frankenpandas-qrn2w`** — `fp-groupby::groupby_sum` silently drops Timedelta64 values.

Three packet families are still red in `cargo test -p fp-conformance` and documented in [`crates/fp-conformance/DISCREPANCIES.md`](crates/fp-conformance/DISCREPANCIES.md):

- **DISC-011 / DISC-014** — Int64 → Float64 promotion on null introduction (pandas nullable-extension behavior we are not yet emulating).
- **DISC-012** — Mixed naive/tz-aware CSV `parse_dates` falls back to raw strings.
- **DISC-013** — Resolved, but its fixture still routes red via DISC-011.

Long-running umbrellas tracked but not blocking:

- **`br-frankenpandas-fd90`** (SQL): SQLite-only today; PostgreSQL/MySQL adapter slices planned. Trait surface is feature-complete.
- **`br-frankenpandas-1zzp`** (Row MultiIndex): closed 2026-04-22; small follow-ups land case-by-case.
- **`br-frankenpandas-4clx`** (Release): 0.1.0 publish to crates.io with signed tag + release-plz automation.
- **`br-frankenpandas-lxhr`** (Monolith split): `fp-conformance/src/lib.rs` reduced from 63,391 → 27,937 lines via 13 test-module extractions; 319 inline tests remain.

## Session metadata (chronicled)

- **2026-04-22** cc-pandas review-mode session (19 passes, 58 beads filed honest-rated). Handoff at [`REVIEW_SESSION_HANDOFF.md`](docs/planning/REVIEW_SESSION_HANDOFF.md).
- **2026-04-22** PANDAS COMPLETE declaration recorded in [`UPGRADE_LOG.md`](docs/planning/UPGRADE_LOG.md).
- **2026-04-23** cc-pandas solo implementation session — supply-chain, release-day metadata, CI polish, community infrastructure, monolith-split cohesive-slice extraction.
- **2026-05-02 → 2026-05-13** distributed agent-swarm sessions (cc-pandas + ntm-orchestrated worker panes) shipped the index-name preservation sweep, typed Index variants, format expansion, and Timedelta64 fast paths.

---

## Evidence sources

- `git log --reverse --format="%h|%ad|%s" --date=short` for chronological history
- `git log --since="..." --until="..." --format="%h %s"` for chunked window research
- `git for-each-ref refs/tags` — 2026-05-16 audit recorded no tags; as of 2026-08-19 the workspace has GitHub Releases `frankenpandas-v0.1.0`, `v0.1.1`, `v0.1.2`, `frankenpandas-v0.2.0` plus crate-level Releases (see Version timeline)
- `gh release list -R Dicklesworthstone/frankenpandas` — Releases exist; do not treat tags `backup/local-main-pre-rebase-80a36bf9` or `coda-fpindex-autostash-backup` as Releases
- `.beads/issues.jsonl` — 1,988 issues, 1,986 closed at the 2026-05-16 audit; 1,910 additional closures in 2026-05-17 → 2026-08-19
- `crates/fp-conformance/DISCREPANCIES.md` — 14 documented divergences (DISC-001 through DISC-014)
- `crates/fp-conformance/fixtures/packets/` — 1,252 packet JSON files
- `find crates -name "*.rs" -type f | xargs wc -l` — line counts
- `grep -rE '^pub (fn|async fn)' crates/*/src/ | wc -l` — public free-fn count
- `grep -rE '#\[(test|tokio::test)\]' crates/*/src/ | wc -l` — test attribute count

CHANGELOG last refreshed **2026-08-19** as a current-window update covering 2026-05-17 → 2026-08-19
(Phase 3 + GitHub Releases + repo-janitor docs move). The 2026-05-16 research pass that built
Phases 1–2 is preserved below that cut. Source-of-truth links use full SHA hashes that resolve
via GitHub redirects. Planning docs live under `docs/planning/` after the 2026-08-19 janitor commits.
