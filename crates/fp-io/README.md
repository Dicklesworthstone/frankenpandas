# fp-io

IO layer for frankenpandas: CSV, JSON, JSONL, Parquet, Excel,
Feather, Arrow IPC, SQL.

Part of the [frankenpandas](https://github.com/Dicklesworthstone/frankenpandas)
workspace.

## Supported formats

| Format | Read | Write | Options |
|--------|:----:|:-----:|---------|
| CSV | `read_csv_str` / `read_csv` | `write_csv_string` | delimiter, na_values, index_col, usecols, nrows, skiprows, dtype |
| JSON | `read_json_str` / `read_json` | `write_json_string` | 5 orients (Records, Columns, Index, Split, Values) |
| JSONL | `read_jsonl_str` / `read_jsonl` | `write_jsonl_string` | One object per line, union-key detection |
| Parquet | `read_parquet_bytes` / `read_parquet` | `write_parquet_bytes` | Arrow RecordBatch integration |
| Excel | `read_excel_bytes` / `read_excel` | `write_excel_bytes` | sheet_name, has_headers, index_col |
| Feather | `read_feather_bytes` | `write_feather_bytes` | Arrow IPC file + stream |
| Arrow IPC | `read_ipc_stream_bytes` | `write_ipc_stream_bytes` | Stream format |
| SQL | `read_sql` / `read_sql_table` | `write_sql` | SqlConnection trait; SQLite today |

SQL backend expansion (PostgreSQL / MySQL) is tracked under
br-frankenpandas-fd90 (slices 2-3 open).

## Native Pickle and HDF snapshots

`write_pickle_bytes` and the file/trait variants write a versioned
`frankenpandas.dataframe.v2` native snapshot. The format retains exact
column dtypes, scalar and missing-value kinds, validity masks, categorical
catalogs, numeric widths, string-extension markers, sparse storage, and
row/column axis metadata. Columns are stored in positional order, including
duplicate column names. HDF stores the same envelope under its configured key.

The reader validates stored columns and axes before exposing the frame.
Unknown format versions and inconsistent state fail with an IO error.
These files are FrankenPandas snapshots, not arbitrary pandas object pickles
or PyTables-compatible HDF tables.

Existing `frankenpandas.dataframe.v1` envelopes remain readable through
their original JSON Split inference. Their absent dtype/null metadata cannot
be reconstructed. New v2 files require a v2-aware reader; older readers refuse
the new marker. Public JSON Split reads and writes retain their existing
pandas-oriented inference independently of the native snapshot format.

## When to depend on fp-io directly

Most users should depend on the umbrella `frankenpandas` crate
which re-exports the IO functions. Direct dependency makes sense
when:

- You need IO primitives but not the full DataFrame arithmetic
  layer.
- You are writing a streaming pipeline that routes through
  `fp-columnar` without materializing a full DataFrame.

## Status

Stable for covered formats. Fuzz regression corpus runs on every
PR (br-frankenpandas-zjme). Differential conformance against live
pandas runs in CI (br-frankenpandas-d6xa). `IoError` is
`#[non_exhaustive]` per br-frankenpandas-tne4.

## Links

- [Workspace README](../../README.md)
- [Security policy](../../SECURITY.md) — IO parsers are the
  primary in-scope surface for vulnerability reports.
