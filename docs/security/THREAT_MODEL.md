# FrankenPandas Per-Subsystem Threat Model

> **Bead:** `br-frankenpandas-rc0923-threat-model-per-subsystem-umlhr`
> **Mandate:** `AGENTS.md` Security Doctrine Minimum Bar #1 — *"Threat model notes for each major subsystem."*
> **Verification Gate:** `crates/fp-conformance/src/tests/ci_supply_chain_policy.rs::threat_model_covers_all_subsystems_with_real_probes_umlhr`

This document defines the attack surface, trust boundaries, concrete mitigations (`file:line`), hostile-input verification probes (unit tests and libFuzzer targets), and open security/hardening gap beads for every major subsystem in FrankenPandas.

---

## Table of Contents

1. [CSV Reader & Writer (`fp-io`)](#1-csv-reader--writer-fp-io)
2. [JSON & JSON-Lines Reader & Writer (`fp-io`)](#2-json--json-lines-reader--writer-fp-io)
3. [Excel Workbook Reader & Writer (`fp-io`)](#3-excel-workbook-reader--writer-fp-io)
4. [Parquet Columnar Reader & Writer (`fp-io`)](#4-parquet-columnar-reader--writer-fp-io)
5. [Arrow IPC / Feather Reader & Writer (`fp-io`)](#5-arrow-ipc--feather-reader--writer-fp-io)
6. [XML Reader & Writer (`fp-io`)](#6-xml-reader--writer-fp-io)
7. [HTML Table Reader & Writer (`fp-io`)](#7-html-table-reader--writer-fp-io)
8. [SAS (`SAS7BDAT` & `XPORT` v5) Reader (`fp-io`)](#8-sas-sas7bdat--xport-v5-reader-fp-io)
9. [Stata DTA Reader & Writer (`fp-io`)](#9-stata-dta-reader--writer-fp-io)
10. [Pickle Snapshot Reader & Writer (`fp-io`)](#10-pickle-snapshot-reader--writer-fp-io)
11. [HDF5 Reader & Writer (`fp-io`)](#11-hdf5-reader--writer-fp-io)
12. [SQL Adapters & Identifier Quoting (`fp-io`)](#12-sql-adapters--identifier-quoting-fp-io)
13. [Expression Parser & Evaluator (`fp-expr`)](#13-expression-parser--evaluator-fp-expr)
14. [Index Alignment & MultiIndex (`fp-index`)](#14-index-alignment--multiindex-fp-index)
15. [Columnar Storage & Vectorized Kernels (`fp-columnar` / `fp-frame` / `fp-groupby` / `fp-join`)](#15-columnar-storage--vectorized-kernels-fp-columnar--fp-frame--fp-groupby--fp-join)
16. [Runtime Policy, Conformal Guard & RaptorQ Envelopes (`fp-runtime`)](#16-runtime-policy-conformal-guard--raptorq-envelopes-fp-runtime)
17. [Python FFI Boundary (`fp-python`)](#17-python-ffi-boundary-fp-python)
18. [Conformance Oracle Subprocess & Fixture Harness (`fp-conformance`)](#18-conformance-oracle-subprocess--fixture-harness-fp-conformance)
19. [Consolidated Security Gap Bead Ledger](#19-consolidated-security-gap-bead-ledger)

---

## 1. CSV Reader & Writer (`fp-io`)

- **Attack Surface:** Untrusted UTF-8/byte streams passed to `read_csv_str` (`crates/fp-io/src/lib.rs:2336`), `read_csv_with_options` (`crates/fp-io/src/lib.rs:6792`), and `read_csv` (`crates/fp-io/src/lib.rs:7337`). Vectors include megabyte-scale single fields, thousands of synthetic columns, unterminated quotes, embedded newlines/NUL bytes inside quotes, integer overflow strings (`> i64::MAX`), and regex delimiter patterns.
- **Trust Boundary:** External file/stream bytes enter `fp-io` before schema inference or `DataFrame` allocation.
- **Mitigations (`file:line`):**
  - Unterminated quote state machine check (`crates/fp-io/src/lib.rs:20813`) rejects unclosed quotes deterministically with `IoError::Csv` rather than over-reading to EOF silently.
  - Typed numeric fast path (`crates/fp-io/src/lib.rs:23276`) falls back cleanly to `Float64` or `Utf8` on integer overflow rather than wrapping `i64`.
  - `#![forbid(unsafe_code)]` across `fp-io` prevents buffer over-reads during `memchr` delimiter scanning.
- **Concrete Hostile Input & Verification Probe:**
  - Hostile input: Unterminated quote in CSV row (`"a,b\n\"unterminated,2\n"`) and 1 MB single CSV cell (`"x".repeat(1_000_000)`).
  - Unit tests: `crates/fp-io/src/lib.rs::csv_unterminated_quote_still_errors_like_pandas` (`crates/fp-io/src/lib.rs:20803`), `crates/fp-io/src/lib.rs::adversarial_csv_very_long_field` (`crates/fp-io/src/lib.rs:33701`), `crates/fp-io/src/lib.rs::adversarial_csv_many_columns` (`crates/fp-io/src/lib.rs:33717`).
  - Fuzz target: `fuzz/fuzz_targets/fuzz_csv_parse.rs`.
- **Known Gaps & Open Beads:**
  - `br-frankenpandas-rc0923-epic-rust-parity-bugs-4qg5w.5` (`read_csv_with_options` with `header=None` and `index_col` drops row 0 or panics on out-of-bounds `index_col`).

---

## 2. JSON & JSON-Lines Reader & Writer (`fp-io`)

- **Attack Surface:** Untrusted JSON documents and NDJSON streams passed to `read_json_str` (`crates/fp-io/src/lib.rs:9235`) and `read_jsonl_str` (`crates/fp-io/src/lib.rs:10746`) across all 6 pandas orientations (`records`, `split`, `index`, `columns`, `values`, `table`). Vectors include deeply nested JSON objects/arrays (`> 100` levels), `i64::MIN`/`i64::MAX` boundary integers, mismatched array lengths in `split`/`columns`, and duplicate column keys.
- **Trust Boundary:** Raw UTF-8 JSON payload parsed by `serde_json` before conversion into `Scalar` vectors and `Index`/`Column` structures.
- **Mitigations (`file:line`):**
  - `serde_json` recursion depth limit (128 levels) prevents stack overflow on deeply nested JSON payloads before `read_json_str` (`crates/fp-io/src/lib.rs:9235`) inspects the value tree.
  - Shape and orientation validation in `read_json_str` (`crates/fp-io/src/lib.rs:9235`) rejects ragged column lengths and non-object/non-array top-level containers with `IoError::Json`.
- **Concrete Hostile Input & Verification Probe:**
  - Hostile input: Deeply nested object values (`{"a": [{"b": {"c": [1, 2, 3]}}]}`) and boundary `i64::MAX` / `i64::MIN` integers in `records` orient.
  - Unit tests: `crates/fp-io/src/lib.rs::adversarial_json_deeply_nested_values` (`crates/fp-io/src/lib.rs:33761`), `crates/fp-io/src/lib.rs::adversarial_json_i64_max_value` (`crates/fp-io/src/lib.rs:33772`), `crates/fp-io/src/lib.rs::adversarial_json_i64_min_value` (`crates/fp-io/src/lib.rs:33783`).
  - Fuzz target: `fuzz/fuzz_targets/fuzz_json_io.rs`.
- **Known Gaps & Open Beads:**
  - `br-frankenpandas-rc0923-epic-rust-parity-bugs-4qg5w.14` (`read_json` `orient=table` silently drops `primaryKey` index metadata and column `schema.fields[].type` coercions).

---

## 3. Excel Workbook Reader & Writer (`fp-io`)

- **Attack Surface:** Untrusted `.xlsx`/`.xls`/`.ods` ZIP-container archives passed to `read_excel_bytes` (`crates/fp-io/src/lib.rs:13298`) and `read_excel` (`crates/fp-io/src/lib.rs:13261`). Vectors include ZIP bombs, malformed worksheet XML (`xl/worksheets/sheet*.xml`), crafted cell references with `>= 7` column letters (e.g., `r="AAAAAAA1"`) that trigger `u32` multiply overflow in `calamine` during base-26 decoding, and out-of-range serial dates.
- **Trust Boundary:** Binary ZIP/XML bytes handed to `read_excel_bytes` prior to `calamine` workbook parsing.
- **Mitigations (`file:line`):**
  - Pre-parse ZIP/Deflate XML cell-reference validation in `validate_xlsx_bytes` (`crates/fp-io/src/lib.rs:13142`) rejects cell references with `> 3` column letters (`> XFD` / `16_384`), row indices `> 1_048_576`, or inverted `<dimension ref="..."/>` ranges with `IoError::Excel` before `calamine` opens the workbook.
  - Explicit width/header validation in `read_excel_bytes_with_index_cols` (`crates/fp-io/src/lib.rs:13324`) rejects mismatched explicit `names` and missing `usecols` targets.
- **Concrete Hostile Input & Verification Probe:**
  - Hostile input: Malformed `.xlsx` archive with out-of-bounds column reference `r="AAAAAAA1"` (`> XFD`, 7 column letters), overflow row `r="99999999"`, inverted `<dimension ref="C5:A1"/>`, or mismatched header width.
  - Unit tests: `crates/fp-io/src/lib.rs::excel_column_overflow_rejects_7_letter_column_and_overflow_row_kyvo0_8` (`crates/fp-io/src/lib.rs:42891`), `crates/fp-io/src/lib.rs::excel_explicit_names_width_mismatch_errors` (`crates/fp-io/src/lib.rs:29052`), `crates/fp-io/src/lib.rs::read_excel_sheets_bytes_unknown_sheet_errors` (`crates/fp-io/src/lib.rs:28686`).
  - Fuzz target: `fuzz/fuzz_targets/fuzz_excel_io.rs`.
- **Known Gaps & Open Beads:**
  - `br-frankenpandas-rc0923-epic-first-green-ci-kyvo0.8` (Guard against `calamine` base-26 `u32` multiply overflow via `validate_xlsx_bytes`).

---

## 4. Parquet Columnar Reader & Writer (`fp-io`)

- **Attack Surface:** Untrusted Apache Parquet byte buffers passed to `read_parquet_bytes` (`crates/fp-io/src/lib.rs:12259`) and `read_parquet` (`crates/fp-io/src/lib.rs:12385`). Vectors include corrupted footer magic (`PAR1`), truncated Thrift metadata, decompression bombs (Zstd/Snappy/Gzip/Brotli/LZ4), dictionary indices pointing out of bounds, and mismatched row-group column lengths.
- **Trust Boundary:** Raw Parquet file bytes parsed into column pages and decoded into `fp-columnar` `Column` buffers.
- **Mitigations (`file:line`):**
  - Footer magic, Thrift schema bounds, and page length checks in `read_parquet_bytes` (`crates/fp-io/src/lib.rs:12259`) fail closed with `IoError::Parquet`.
  - Dictionary page validation in `parquet_reader_uses_nullable_utf8_buffer_and_rejects_dictionary_uza04` (`crates/fp-io/src/lib.rs:27466`) rejects invalid dictionary encodings cleanly.
- **Concrete Hostile Input & Verification Probe:**
  - Hostile input: Truncated Parquet header/footer bytes and malformed dictionary-encoded UTF-8 page buffer.
  - Unit tests: `crates/fp-io/src/lib.rs::parquet_reader_uses_nullable_utf8_buffer_and_rejects_dictionary_uza04` (`crates/fp-io/src/lib.rs:27466`), `crates/fp-io/src/lib.rs::parquet_empty_dataframe_errors` (`crates/fp-io/src/lib.rs:27739`).
  - Fuzz target: `fuzz/fuzz_targets/fuzz_parquet_io.rs`.
- **Known Gaps & Open Beads:**
  - `br-frankenpandas-rc0923-epic-full-surface-conformance-j0lg5.6` (Cross-engine PyArrow/fastparquet differential fuzzing across nested Parquet logical types).

---

## 5. Arrow IPC / Feather Reader & Writer (`fp-io`)

- **Attack Surface:** Untrusted Arrow IPC file (`ARROW1`) and stream buffers passed to `read_feather_bytes` (`crates/fp-io/src/lib.rs:13416`) and `read_ipc_stream_bytes` (`crates/fp-io/src/lib.rs:13495`). Vectors include invalid FlatBuffer offsets, negative buffer lengths, LZ4/Zstd decompression amplification, and temporal unit conversion (`s`/`ms`/`us` to `ns`) `i64` multiplication overflow.
- **Trust Boundary:** Binary Arrow IPC messages and body buffers prior to constructing `Column` and `Index` values.
- **Mitigations (`file:line`):**
  - Checked arithmetic during temporal unit widening in `read_feather_bytes` (`crates/fp-io/src/lib.rs:13416`) returns `IoError` on `i64` nanosecond overflow rather than wrapping timestamps.
  - Buffer slice bounds validation rejects truncated body buffers before copying into `ValidityMask` or typed vectors.
- **Concrete Hostile Input & Verification Probe:**
  - Hostile input: Arrow IPC `Timestamp(Second)` / `Duration(Second)` buffer containing `i64::MAX` (which overflows `i64` when multiplied by `1_000_000_000` for nanoseconds).
  - Unit tests: `crates/fp-io/src/lib.rs::arrow_temporal_units_widen_to_nanoseconds_and_overflow_is_an_error` (`crates/fp-io/src/lib.rs:32441`), `crates/fp-io/src/lib.rs::feather_reader_decodes_lz4_compressed_ipc_like_pyarrow_writes` (`crates/fp-io/src/lib.rs:32239`).
  - Fuzz targets: `fuzz/fuzz_targets/fuzz_feather_io.rs`, `fuzz/fuzz_targets/fuzz_ipc_stream_io.rs`.
- **Known Gaps & Open Beads:**
  - `br-frankenpandas-rc0923-epic-full-surface-conformance-j0lg5.6` (Expand cross-format IPC stream differential corpus).

---

## 6. XML Reader & Writer (`fp-io`)

- **Attack Surface:** Untrusted XML strings/files passed to `read_xml_str` (`crates/fp-io/src/lib.rs:4594`) and `read_xml` (`crates/fp-io/src/lib.rs:7869`). Vectors include billion-laughs entity expansion (`<!ENTITY>`), mismatched closing tags, deeply nested elements inside record nodes, and duplicate child field tags within a single row element.
- **Trust Boundary:** Raw XML text entering the structural XML tokenizer in `fp-io`.
- **Mitigations (`file:line`):**
  - Custom non-expanding XML parser in `read_xml_str_with_options` (`crates/fp-io/src/lib.rs:4599`) never fetches external DTDs/XXE URIs and rejects nested complex elements or duplicate field names within a record with `IoError::Xml`.
- **Concrete Hostile Input & Verification Probe:**
  - Hostile input: Malformed XML with unclosed tags, nested sub-records `<row><a><nested>1</nested></a></row>`, or duplicate sibling tags `<row><a>1</a><a>2</a></row>`.
  - Unit test: `crates/fp-io/src/lib.rs::xml_reader_rejects_malformed_nested_and_duplicate_fields` (`crates/fp-io/src/lib.rs:23131`).
  - Fuzz target: `fuzz/fuzz_targets/fuzz_format_cross_round_trip.rs`.
- **Known Gaps & Open Beads:**
  - `br-frankenpandas-rc0923-epic-full-surface-conformance-j0lg5.6` (XPath subset parity and namespace prefix edge cases).

---

## 7. HTML Table Reader & Writer (`fp-io`)

- **Attack Surface:** Untrusted HTML documents passed to `read_html_str` (`crates/fp-io/src/lib.rs:3874`) and `to_html` string rendering. Vectors include XSS payloads (`<script>`, `onload=`) in DataFrame cells when rendered to HTML, documents with no `<table>` tags, duplicate `<th>` header labels, and ragged `<tr>` rows with more `<td>` cells than headers.
- **Trust Boundary:** Untrusted HTML text on ingestion (`read_html_str`) and untrusted cell strings on emission (`to_html`).
- **Mitigations (`file:line`):**
  - HTML entity escaping (`&`, `<`, `>`, `"`, `'`) is enabled by default (`escape = true`) in the HTML table writer (`crates/fp-io/src/lib.rs:21666`).
  - `read_html_str_with_options` (`crates/fp-io/src/lib.rs:3879`) rejects tables with duplicate headers or rows wider than the header schema with `IoError::Html`.
- **Concrete Hostile Input & Verification Probe:**
  - Hostile input: HTML `<table>` containing duplicate `<th>a</th><th>a</th>` headers or wide `<tr><td>1</td><td>2</td><td>3</td></tr>` rows; cells containing `<script>alert(1)</script>`.
  - Unit tests: `crates/fp-io/src/lib.rs::html_reader_rejects_no_table_duplicate_headers_and_wide_rows` (`crates/fp-io/src/lib.rs:21898`), `crates/fp-io/src/lib.rs::html_table_writer_supports_pandas_pure_string_options_u892h` (`crates/fp-io/src/lib.rs:21666`).
  - Fuzz target: `fuzz/fuzz_targets/fuzz_format_cross_round_trip.rs`.
- **Known Gaps & Open Beads:**
  - `br-frankenpandas-rc0923-epic-full-surface-conformance-j0lg5.6` (`colspan`/`rowspan` grid expansion parity in `read_html`).

---

## 8. SAS (`SAS7BDAT` & `XPORT` v5) Reader (`fp-io`)

- **Attack Surface:** Untrusted binary SAS transport (`XPORT` v5) and native `SAS7BDAT` files passed to `read_sas` (`crates/fp-io/src/lib.rs:7578`). Vectors include truncated 80-byte XPORT card headers, invalid IBM floating-point exponents, RLE decompression bombs in `SAS7BDAT` pages, undecoded trailing bytes, and ragged column cell counts.
- **Trust Boundary:** Binary SAS file bytes read from disk or memory buffer before constructing typed `Series`.
- **Mitigations (`file:line`):**
  - Header signature, page boundary, and RLE output length validation in `read_sas` (`crates/fp-io/src/lib.rs:7578`) reject truncated or undecoded byte streams with `IoError::Sas`.
  - Frame assembly checks that every parsed column has identical row count and zero undecoded cells.
- **Concrete Hostile Input & Verification Probe:**
  - Hostile input: SAS record containing undecoded cell bytes, truncated `.sas7bdat` / `.xpt` files, or ragged column lengths across variables.
  - Unit tests: `crates/fp-io/src/lib.rs::test_real_sas7bdat_and_xport_differential_against_pandas_j0lg5_8` (`crates/fp-io/src/lib.rs:43043`), `crates/fp-io/src/lib.rs::test_truncated_sas7bdat_and_xport_return_typed_io_error_j0lg5_8` (`crates/fp-io/src/lib.rs:43235`), `crates/fp-io/src/lib.rs::sas_cells_build_typed_frame_and_reject_undecoded_bytes_9jvao` (`crates/fp-io/src/lib.rs:27500`), `crates/fp-io/src/lib.rs::sas_frame_rejects_ragged_parser_output_9jvao` (`crates/fp-io/src/lib.rs:27532`).
  - Fuzz target: `fuzz/fuzz_targets/fuzz_format_cross_round_trip.rs`.
- **Known Gaps & Open Beads:**
  - `br-frankenpandas-rc0923-epic-full-surface-conformance-j0lg5.8` (Differential parity tests against `pandas.read_sas` on canonical binary `.sas7bdat` and `.xpt` fixtures plus truncated-file negative probes).

---

## 9. Stata DTA Reader & Writer (`fp-io`)

- **Attack Surface:** Untrusted Stata `.dta` binary files passed to `read_stata_bytes` (`crates/fp-io/src/lib.rs:4208`) and `read_stata` (`crates/fp-io/src/lib.rs:10662`), plus untrusted column names passed to `write_stata_bytes`. Vectors include corrupted `<stata_dta>` XML-like format-118 headers, invalid variable type codes, non-ASCII/overlong variable names (`> 32` chars), and truncated observation rows.
- **Trust Boundary:** Binary `.dta` stream on read; caller-supplied `DataFrame` column names on write.
- **Mitigations (`file:line`):**
  - Strict header tag, byteorder, and observation-length verification in `read_stata_bytes` (`crates/fp-io/src/lib.rs:4208`).
  - Stata variable name validator rejects empty names, leading digits, non-identifier characters, and names exceeding 32 bytes before writing `.dta` headers.
- **Concrete Hostile Input & Verification Probe:**
  - Hostile input: Truncated/malformed `.dta` byte slice `b"not a stata file"` and invalid Stata variable name `"1bad-name"`.
  - Unit test: `crates/fp-io/src/lib.rs::stata_writer_rejects_invalid_variable_names_and_malformed_input` (`crates/fp-io/src/lib.rs:23313`).
  - Fuzz target: `fuzz/fuzz_targets/fuzz_format_cross_round_trip.rs`.
- **Known Gaps & Open Beads:**
  - `br-frankenpandas-rc0923-epic-full-surface-conformance-j0lg5.6` (Legacy Stata 114/115/117 format version reader parity).

---

## 10. Pickle Snapshot Reader & Writer (`fp-io`)

- **Attack Surface:** Untrusted `.pkl` payloads passed to `read_pickle_bytes` (`crates/fp-io/src/lib.rs:3987`) and `read_pickle` (`crates/fp-io/src/lib.rs:10480`), or via Python `fp.read_pickle` (`crates/fp-python/src/lib.rs:101682`). In CPython/pandas, `pickle.loads` executes arbitrary code via `GLOBAL`/`STACK_GLOBAL`/`REDUCE` opcodes.
- **Trust Boundary:** Untrusted byte stream claiming to be a serialized DataFrame/Series pickle.
- **Mitigations (`file:line`):**
  - **No arbitrary Python opcode execution in Rust:** `read_pickle_bytes` (`crates/fp-io/src/lib.rs:3987`) is a restricted structural parser that accepts only FrankenPandas snapshot envelopes and known safe DataFrame/Series structural opcodes; any foreign `GLOBAL`/`REDUCE` class invocation is rejected with `IoError::Pickle`.
- **Concrete Hostile Input & Verification Probe:**
  - Hostile input: Malformed pickle stream and foreign Python class `GLOBAL` payload (`cos\nsystem\n...`).
  - Unit test: `crates/fp-io/src/lib.rs::pickle_reader_rejects_malformed_and_foreign_payloads` (`crates/fp-io/src/lib.rs:22824`).
  - Fuzz target: `fuzz/fuzz_targets/fuzz_format_cross_round_trip.rs`.
- **Known Gaps & Open Beads:**
  - `br-frankenpandas-rc0923-epic-full-surface-conformance-j0lg5.6` (Pure-Rust decoding of additional NumPy/pandas v2 BlockManager pickle opcodes without delegating to Python `pickle.loads`).

---

## 11. HDF5 Reader & Writer (`fp-io`)

- **Attack Surface:** Untrusted `.h5`/`.hdf5` files and group keys passed to `read_hdf` (`crates/fp-io/src/lib.rs:10512`) and `read_hdf_with_options` (`crates/fp-io/src/lib.rs:10528`). Vectors include path-traversal/empty HDF5 group keys (`""`, `"/"`), corrupted superblocks, and missing dataset attributes.
- **Trust Boundary:** External HDF5 file bytes and caller-provided group `key` strings.
- **Mitigations (`file:line`):**
  - Key normalization and validation in `read_hdf_with_options` (`crates/fp-io/src/lib.rs:10528`) rejects empty or root-only keys and verifies dataset schema metadata before materializing columns.
- **Concrete Hostile Input & Verification Probe:**
  - Hostile input: Empty HDF5 key `""`, nonexistent group key `"/missing"`, or corrupted HDF5 payload bytes.
  - Unit test: `crates/fp-io/src/lib.rs::hdf5_reader_rejects_invalid_keys_and_missing_payloads` (`crates/fp-io/src/lib.rs:22946`).
  - Fuzz target: `fuzz/fuzz_targets/fuzz_format_cross_round_trip.rs`.
- **Known Gaps & Open Beads:**
  - `br-frankenpandas-rc0923-epic-full-surface-conformance-j0lg5.6` (PyTables `table` format query predicate pushdown parity).

---

## 12. SQL Adapters & Identifier Quoting (`fp-io`)

- **Attack Surface:** Table names, schema names, and column names passed to `read_sql_table` (`crates/fp-io/src/lib.rs:18085`), `write_sql`, and `SqlConnection::quote_identifier` across SQLite (`SqliteConnection`, `crates/fp-io/src/lib.rs:14818`), PostgreSQL (`PostgresConnection`, `crates/fp-io/src/lib.rs:16619`), and MySQL (`MysqlConnection`, `crates/fp-io/src/lib.rs:17311`). Vectors include SQL injection via table/column identifiers containing embedded quotes (`"`, `` ` ``), semicolons (`users"; DROP TABLE users; --`), or NUL bytes (`\0`), as well as `TIMESTAMPTZ` vs `TIMESTAMP` timezone drift across SQL read/write boundaries.
- **Trust Boundary:** Caller-supplied table/schema/column identifiers and `Scalar` row values crossing into SQL query strings and wire parameters.
- **Mitigations (`file:line`):**
  - Dialect-aware `quote_identifier` (`crates/fp-io/src/lib.rs:14818`, `15348`, `15780`, `17311`) rejects any identifier containing `\0` (`IoError::Sql`) and doubles embedded quote delimiters (`"` -> `""` for ANSI/SQLite/PostgreSQL; `` ` `` -> `` `` `` for MySQL).
  - PostgreSQL OID-aware `pg_type_oid_to_dtype` (`crates/fp-io/src/lib.rs:16609`) and `postgres_dtype_sql` (`crates/fp-io/src/lib.rs:16585`) distinguish `TIMESTAMP` (OID 1114, `tz: None`) from `TIMESTAMPTZ` (OID 1184, `tz: Some("UTC")`).
  - Data values in `write_sql` / parameterized queries use bound parameters or dialect-safe literal formatting rather than unquoted string interpolation.
- **Concrete Hostile Input & Verification Probe:**
  - Hostile input: Table/column identifiers containing NUL bytes (`"bad\0ident"`), embedded double quotes (`r#"col"name"#`), embedded backticks (`` "col`name" ``), SQL reserved words (`"select"`, `"table"`), and mixed `TIMESTAMP` / `TIMESTAMPTZ` columns.
  - Unit tests: `crates/fp-io/src/lib.rs::fd90_12_quoting_matrix_nul_byte_rejected_at_quote_identifier_layer` (`crates/fp-io/src/lib.rs:41479`), `crates/fp-io/src/lib.rs::fd90_12_quoting_matrix_embedded_quote_chars_doubled_per_dialect` (`crates/fp-io/src/lib.rs:41322`), `crates/fp-io/src/lib.rs::rusqlite_quote_identifier_rejects_null_bytes` (`crates/fp-io/src/lib.rs:34579`), `crates/fp-io/src/lib.rs::adversarial_sql_column_name_with_quotes_accepted` (`crates/fp-io/src/lib.rs:34408`), `crates/fp-io/src/lib.rs::test_postgres_live_timestamptz_vs_timestamp_roundtrip_4qg5w_15` (`crates/fp-io/src/lib.rs:30436`).
  - Fuzz target: `fuzz/fuzz_targets/fuzz_sql_read.rs`.
- **Known Gaps & Open Beads:**
  - `br-frankenpandas-rc0923-epic-rust-parity-bugs-4qg5w.15` (PostgreSQL `TIMESTAMPTZ` OID 1184 vs `TIMESTAMP` OID 1114 UTC tz-awareness parity on read/write).

---

## 13. Expression Parser & Evaluator (`fp-expr`)

- **Attack Surface:** Untrusted expression strings passed to `parse_expr` (`crates/fp-expr/src/lib.rs:2233`), `evaluate` (`crates/fp-expr/src/lib.rs:688`), and `query_str` (`crates/fp-expr/src/lib.rs:1369`). Vectors include deeply nested parentheses (`((((...))))`) aiming to blow the call stack, string repetition bombs (`"a" * 100_000_000` in hardened mode), malformed floating-point literals (`1.2.3`), and unterminated backtick identifiers.
- **Trust Boundary:** Untrusted user query/eval strings entering the recursive-descent parser and eager columnar evaluator.
- **Mitigations (`file:line`):**
  - Explicit recursion-depth cap in `parse_expr` (`crates/fp-expr/src/lib.rs:2233`) rejects excessive nesting with `ExprError::Parse` before stack exhaustion.
  - Hardened-mode string repetition cap in `evaluate` (`crates/fp-expr/src/lib.rs:10840`) bounds repeated string allocation sizes.
- **Concrete Hostile Input & Verification Probe:**
  - Hostile input: Deeply nested parenthesized expression `"(".repeat(256) + "1" + &")".repeat(256)` and hardened-mode string repeat bomb.
  - Unit tests: `crates/fp-expr/src/lib.rs::nesting_pandas_evaluates_works_and_deeper_is_a_parse_error_bhbuz` (`crates/fp-expr/src/lib.rs:10429`), `crates/fp-expr/src/lib.rs::hardened_mode_bounds_a_repeated_string` (`crates/fp-expr/src/lib.rs:10840`), `crates/fp-expr/src/lib.rs::parse_error_unterminated_backtick_identifier` (`crates/fp-expr/src/lib.rs:6900`).
  - Fuzz targets: `fuzz/fuzz_targets/fuzz_parse_expr.rs`, `fuzz/fuzz_targets/fuzz_dataframe_eval.rs`, `fuzz/fuzz_targets/fuzz_query_str.rs`, `fuzz/fuzz_targets/fuzz_query_str_with_locals.rs`.
- **Known Gaps & Open Beads:**
  - `br-frankenpandas-rc0923-epic-full-surface-conformance-j0lg5.5` (Expand `eval`/`query` differential packet coverage for chained boolean/bitwise expressions).

---

## 14. Index Alignment & MultiIndex (`fp-index`)

- **Attack Surface:** Untrusted or adversarial index label sequences and `MultiIndex` level cardinalities passed to `Index` / `MultiIndex` constructors and alignment plans (`crates/fp-index/src/lib.rs`). Vectors include Cartesian-product cardinality overflow in `MultiIndex::from_product` (`level_len_1 * level_len_2 * ... > usize::MAX`), `Index::repeat` count overflow, `date_range` nanosecond timestamp overflow, and high-collision duplicate keys in hash alignment.
- **Trust Boundary:** Index label vectors and repetition/product counts prior to allocating index buffers or alignment maps.
- **Mitigations (`file:line`):**
  - Checked capacity multiplication in `MultiIndex::from_product` (`crates/fp-index/src/lib.rs:37066`) and `Index::repeat` / `insert` / `append` (`crates/fp-index/src/lib.rs:43016`) returns `IndexError` instead of panicking or wrapping on `usize` overflow.
  - Checked timestamp progression in `date_range` (`crates/fp-index/src/lib.rs:27834`) rejects nanosecond `i64` overflow.
- **Concrete Hostile Input & Verification Probe:**
  - Hostile input: `MultiIndex` product levels whose product overflows `usize::MAX`, and `Index::repeat(usize::MAX)`.
  - Unit tests: `crates/fp-index/src/lib.rs::multi_index_product_cardinality_rejects_overflow_uza04184` (`crates/fp-index/src/lib.rs:37066`), `crates/fp-index/src/lib.rs::index_repeat_capacity_rejects_overflow_uza04182` (`crates/fp-index/src/lib.rs:43016`), `crates/fp-index/src/lib.rs::date_range_rejects_generated_timestamp_overflow` (`crates/fp-index/src/lib.rs:27834`).
  - Fuzz target: `fuzz/fuzz_targets/fuzz_index_align.rs`.
- **Known Gaps & Open Beads:**
  - `br-frankenpandas-rc0923-epic-full-surface-conformance-j0lg5.1` (Include `IntervalIndex` in the automated API surface denominator and drift scripts).

---

## 15. Columnar Storage & Vectorized Kernels (`fp-columnar` / `fp-frame` / `fp-groupby` / `fp-join`)

- **Attack Surface:** Untrusted numeric/temporal arrays, window bounds, slice lengths, and join keys across `fp-columnar`, `fp-frame`, `fp-groupby`, and `fp-join`. Vectors include `i64::MIN` negation/subtraction overflow in `Timedelta64` / `Datetime64` kernels, huge slice lengths (`usize::MAX`), HyperLogLog high-rho bit patterns in `fp-groupby`, and many-to-many join blowups in `fp-join`.
- **Trust Boundary:** Columnar kernel entry points operating on typed slices and `ValidityMask` bitmaps.
- **Mitigations (`file:line`):**
  - Temporal arithmetic overflow in `fp-columnar` (`crates/fp-columnar/src/lib.rs:54930`, `55057`) masks overflowed cells as `NaT` (`ValidityMask` bit cleared) rather than fabricating `i64::MAX` or panicking in debug mode.
  - Slice clamping (`crates/fp-columnar/src/lib.rs:49968`) saturates out-of-range slice lengths without `usize` addition overflow.
  - HyperLogLog register update in `fp-groupby` (`crates/fp-groupby/src/lib.rs:7987`) bounds shift amounts when `rho` reaches 64.
- **Concrete Hostile Input & Verification Probe:**
  - Hostile input: Two-column `Timedelta64` subtraction at `i64::MIN - i64::MAX` and `slice(10, usize::MAX)`.
  - Unit tests: `crates/fp-columnar/src/lib.rs::diff_timedelta64_overflow_is_nat_not_fabricated_max_zffwp` (`crates/fp-columnar/src/lib.rs:54930`), `crates/fp-columnar/src/lib.rs::temporal_two_col_add_overflow_is_masked_missing_o9e5q` (`crates/fp-columnar/src/lib.rs:55057`), `crates/fp-columnar/src/lib.rs::slice_huge_len_clamps_without_overflow` (`crates/fp-columnar/src/lib.rs:49968`), `crates/fp-groupby/src/lib.rs::hll_high_rho_does_not_overflow` (`crates/fp-groupby/src/lib.rs:7987`).
  - Fuzz targets: `fuzz/fuzz_targets/fuzz_column_arith.rs`, `fuzz/fuzz_targets/fuzz_groupby_sum.rs`, `fuzz/fuzz_targets/fuzz_join_series.rs`, `fuzz/fuzz_targets/fuzz_rolling_window.rs`.
- **Known Gaps & Open Beads:**
  - `br-frankenpandas-rc0923-epic-buildable-everywhere-0zz8y.6` (Ensure `fp-dot-kernel` uses a compile-time `#[cfg(all(target_feature = "avx2", target_feature = "fma"))]` token rather than runtime `is_x86_feature_detected!` branching inside hot loops).

---

## 16. Runtime Policy, Conformal Guard & RaptorQ Envelopes (`fp-runtime`)

- **Attack Surface:** Adversarial cardinality estimates, corrupted sidecar bytes, and bit-flipped repair symbols passed to `RuntimePolicy` (`crates/fp-runtime/src/lib.rs:370`), `RaptorQEnvelope` (`crates/fp-runtime/src/lib.rs:538`), and `ConformalGuard` (`crates/fp-runtime/src/lib.rs:739`). Vectors include `usize::MAX` estimated join rows in Bayesian loss evaluation, truncated or bit-flipped RaptorQ source packets, and non-finite calibration scores.
- **Trust Boundary:** Runtime policy decision boundaries and durable artifact verification (`RaptorQEnvelope::verify_and_recover`).
- **Mitigations (`file:line`):**
  - Hardened `RuntimePolicy` (`crates/fp-runtime/src/lib.rs:370`) caps join/allocation admissions when `estimated_rows > hardened_join_row_cap`, recording a deterministic `DecisionRecord` in the `EvidenceLedger`.
  - `RaptorQEnvelope` (`crates/fp-runtime/src/lib.rs:538`) verifies BLAKE3/SHA-256 content hashes before and after RFC 6330 FEC repair, emitting a cryptographic `DecodeProof` or failing closed if corruption exceeds repair symbols.
- **Concrete Hostile Input & Verification Probe:**
  - Hostile input: Extreme join cardinality estimate (`usize::MAX`) under hardened mode and corrupted source symbol bytes in a `RaptorQEnvelope`.
  - Unit tests: `crates/fp-runtime/src/lib.rs::asupersync_adversarial_extreme_join_estimate_remains_repair_and_loggable` (`crates/fp-runtime/src/lib.rs:1825`), `crates/fp-runtime/src/lib.rs::raptorq_envelope_from_source_bytes_invariants_b9vvk` (`crates/fp-runtime/src/lib.rs:2010`), `crates/fp-runtime/src/lib.rs::conformal_guard_calibrates_after_sufficient_data` (`crates/fp-runtime/src/lib.rs:2204`).
  - Fuzz target: `fuzz/fuzz_targets/fuzz_dataframe_op_chain.rs`.
- **Known Gaps & Open Beads:**
  - `br-frankenpandas-rc0923-epic-full-surface-conformance-j0lg5.7` (Extend automated RaptorQ scrub verification across newly added benchmark baseline bundles).

---

## 17. Python FFI Boundary (`fp-python`)

- **Attack Surface:** Arbitrary Python objects (`PyAny`), oversized Python integers (`> 64-bit` arbitrary-precision `int`), non-UTF-8 `bytes`/surrogates, and buffer-protocol views crossing the PyO3 FFI boundary in `crates/fp-python/src/lib.rs`. Vectors include 128-bit Python `int` values overflowing `i64` during arithmetic/comparison, out-of-range `datetime.timedelta` microsecond counts, and hostile `read_pickle` paths.
- **Trust Boundary:** Every `#[pymethods]` / `#[pyfunction]` entry point in `crates/fp-python/src/lib.rs` converting `Bound<'_, PyAny>` into Rust `Scalar`, `Series`, or `DataFrame`.
- **Mitigations (`file:line`):**
  - `refuse_int_overflow` (`crates/fp-python/src/lib.rs:30464`) and `duration_overflow` (`crates/fp-python/src/lib.rs:5012`) catch Python integers and timedeltas that overflow `i64` and raise `OverflowError` matching pandas/NumPy rather than silently truncating or panicking across FFI.
  - `#![forbid(unsafe_code)]` in `fp-python` ensures all PyO3 GIL and object conversions use safe `Bound<'py, T>` handles.
- **Concrete Hostile Input & Verification Probe:**
  - Hostile input: Python `int` exceeding `i64::MAX` (`10**30`) passed to `Series` comparison/arithmetic or CSV ingestion, and malformed indexer keys.
  - Unit tests: `crates/fp-python/src/lib.rs::test_py_dataframe_indexers_and_dunders` (`crates/fp-python/src/lib.rs:102298`), `crates/fp-python/tests/test_differential_packets.py`.
  - Fuzz target: `fuzz/fuzz_targets/fuzz_scalar_cast.rs`.
- **Known Gaps & Open Beads:**
  - `br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao` (Audit remaining `fp-python` methods to ensure zero silent fallback or empty-result fabrication on unsupported keyword arguments).

---

## 18. Conformance Oracle Subprocess & Fixture Harness (`fp-conformance`)

- **Attack Surface:** JSON conformance fixture packets under `crates/fp-conformance/fixtures/packets/*.json` and live-oracle subprocess execution (`run_live_oracle_report`, `crates/fp-conformance/src/lib.rs:6451`). Vectors include malformed fixture JSON, corrupt sidecar `.raptorq.json` bundles, adversarial fuzz-corpus seeds (`crates/fp-conformance/src/tests/fuzz_seed_fixtures.rs`), and untrusted environment variables (`FP_PYTHON_BIN`).
- **Trust Boundary:** JSON fixture deserialization and `std::process::Command` invocation of `crates/fp-conformance/oracle/pandas_oracle.py`.
- **Mitigations (`file:line`):**
  - `Command::new(&config.python_bin)` in `run_live_oracle_report` (`crates/fp-conformance/src/lib.rs:6451`) passes arguments as discrete OS `argv` elements (never via `sh -c`) and communicates with `pandas_oracle.py` over structured JSON stdin/stdout.
  - Fault-injection validation (`run_fault_injection_validation_by_id`, `crates/fp-conformance/src/lib.rs:4803`) verifies strict fail-closed rejection vs bounded hardened recovery on corrupted fixtures.
- **Concrete Hostile Input & Verification Probe:**
  - Hostile input: Corrupted fixture payload injected via `run_fault_injection_validation_by_id` and truncated JSON seed `br#"{"packet_id": "oops""#` in `crates/fp-conformance/src/tests/fuzz_seed_fixtures.rs`.
  - Unit tests: `crates/fp-conformance/src/lib.rs::fault_injection_validation_classifies_strict_vs_hardened` (`crates/fp-conformance/src/lib.rs:27035`), `crates/fp-conformance/src/tests/fuzz_seed_fixtures.rs::fuzz_fixture_parse_bytes_reports_invalid_json` (`crates/fp-conformance/src/tests/fuzz_seed_fixtures.rs:29`).
  - Fuzz target: `fuzz/fuzz_targets/fuzz_fixture_parse.rs`.
- **Known Gaps & Open Beads:**
  - `br-frankenpandas-rc0923-epic-full-surface-conformance-j0lg5.1` (Honest full-surface conformance denominator across all 16 pandas classes).


---

## 19. Consolidated Security Gap Bead Ledger

| Gap Bead ID | Subsystem | Summary |
|---|---|---|
| `br-frankenpandas-rc0923-epic-first-green-ci-kyvo0.8` | Excel Reader (`fp-io`) | Guard against `calamine` base-26 `u32` multiply overflow on `.xlsx` cell references with `> 3` column letters (e.g., `AAAAAAA1`). |
| `br-frankenpandas-rc0923-epic-rust-parity-bugs-4qg5w.5` | CSV Reader (`fp-io`) | Prevent row-0 loss or out-of-bounds panic when `header=None` is combined with `index_col` in `read_csv_with_options`. |
| `br-frankenpandas-rc0923-epic-rust-parity-bugs-4qg5w.14` | JSON Reader (`fp-io`) | Preserve `primaryKey` index and `schema.fields` dtypes in `read_json` `orient=table`. |
| `br-frankenpandas-rc0923-epic-rust-parity-bugs-4qg5w.15` | SQL PostgreSQL (`fp-io`) | Preserve `TIMESTAMPTZ` (OID 1184) vs `TIMESTAMP` (OID 1114) UTC timezone metadata across PostgreSQL read/write boundaries. |
| `br-frankenpandas-rc0923-epic-full-surface-conformance-j0lg5.8` | SAS Reader (`fp-io`) | Add real-binary `sas7bdat` and `xport` differential parity and truncated-binary rejection tests against `pandas.read_sas`. |
| `br-frankenpandas-rc0923-epic-buildable-everywhere-0zz8y.6` | SIMD Kernels (`fp-dot-kernel`) | Replace runtime `is_x86_feature_detected!` checks with compile-time `Avx2FmaToken` (`#[cfg(all(target_feature = "avx2", target_feature = "fma"))]`). |
| `br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao` | Python FFI (`fp-python`) | Ensure unsupported kwargs or edge cases raise explicit Python exceptions rather than returning fabricated empty outputs. |
| `br-frankenpandas-rc0923-epic-full-surface-conformance-j0lg5.1` | Conformance (`fp-conformance`) | Track all 16 top-level pandas classes (including `IntervalIndex`) in the automated API surface coverage ledger. |
