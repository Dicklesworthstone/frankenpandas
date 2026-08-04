# uza04.112 — reindex parallel per-column gather (+unit-range positions) — KEPT (~7.4x, pandas parity)

BlackThrush, 2026-06-13. fp-bench vs-pandas (indexing).

## Target
After uza04.110 (positions-once + typed gather), reindex 100k x10 float64 sat at
~8 ms vs pandas 1.11 ms (~7x). Phase reasoning: label resolution is negligible;
the per-column typed gather (10 cols × 100k) dominates.

## Lever (one, bit-identical)
- The per-column gather is independent across columns -> fan across workers
  (work-stealing thread::scope), gated on new_len·ncols >= 2^18.
- Bonus (bit-identical, neutral alone): unit-range Int64 index resolves Int64(v)
  -> v-start arithmetically (no FxHashMap) — pandas RangeIndex default.
Each column's typed Float64 gather is unchanged (present -> data[idx]+set bit;
missing -> 0.0+cleared bit => Null(NaN)).

## Proof
- `cargo test -p fp-frame reindex`: 28 passed.
- fp-bench reindex 100k: **~8 -> ~1.1 ms p50 (min 1.0) = ~7.4x** — now at pandas
  parity (1.11 ms). Total from original 65.2ms: ~59x.

## Next swing
Same parallel-per-column-gather lever applies to `loc_with_columns` (~8ms, ~4x vs
pandas) — its gather is also independent per column.
