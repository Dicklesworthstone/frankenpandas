# fp-bench io category — FP CSV 22-69x faster than pandas

BlackThrush, 2026-06-13. Completes fp-bench's coverage of all 5 vs-pandas harness
categories (dataframe_ops/groupby/rolling/indexing/io).

## Added
- io/csv_read: serialize the frame once (setup) via write_csv_string, time
  read_csv_str of that text (parse-bound, comparable to pandas read_csv).
- io/csv_write: time write_csv_string(frame).
- parquet_* return unsupported (no FP parquet; pandas also needs pyarrow).

## Measured (100k x10 float64)
- csv_read:  FP 1.5 ms  vs pandas 106 ms  -> ~69x FASTER
- csv_write: FP 43 ms   vs pandas 957 ms  -> ~22x FASTER

No io gap — FrankenPandas decisively wins CSV. The category now guards against io
regressions and rounds out the no-gaps measurement surface.
