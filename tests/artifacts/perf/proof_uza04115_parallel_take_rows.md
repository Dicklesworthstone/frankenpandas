# uza04.114 — parallel per-column take_rows gather — KEPT (~2.2x, beats pandas)

BlackThrush, 2026-06-13. fp-bench vs-pandas.

## Target
filter_bool_mask @ 100k float64: fp 3.18 ms vs pandas 1.6 ms = 1.99x SLOWER.
loc_bool -> take_rows_by_positions gathered columns SERIALLY (each
`take_positions` is typed, but the loop over columns was single-threaded). Shared
by boolean filter / loc_bool / iloc_bool / positional take.

## Lever (one, bit-identical)
Fan the per-column `take_positions` gather across workers (work-stealing
thread::scope, gated n·ncols >= 2^18). Each column's gather is unchanged; only the
owning thread differs, so columns/order are byte-identical.

## Proof
- Golden sha256 filter_bool 100000 = 2f77640d… — matches the established golden
  (uza04.76); 2000 = 75dcc7b5… Bit-identical.
- Tests: loc_bool 15, filter 29, iloc 13, take 29 — all pass.
- fp-bench filter_bool_mask 100k: **3.18 -> ~1.4 ms p50 (min 1.1) = ~2.2x** — now
  FASTER than pandas (1.6 ms), gap closed.
