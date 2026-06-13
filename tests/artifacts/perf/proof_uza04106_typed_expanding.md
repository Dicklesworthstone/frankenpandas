# uza04.106 — typed f64 expanding sum/mean fast path — KEPT (~10.6x)

BlackThrush, 2026-06-13. Gap via fp-bench vs-pandas harness.

## Target
expanding_sum @ 100k float64: fp 4.46 ms vs pandas 0.79 ms = 5.63x SLOWER.
`running_sum` (shared by expanding sum/mean) is O(n) but materialized `.values()`
(Scalar per row) + `Vec<Scalar>` / `Column::from_values` re-scan.

## Lever (one, bit-identical)
When `min_periods <= 1` and all-valid (`as_f64_slice` ⇒ validity.all() + NaN-free),
the running count reaches min_periods at row 0, so no output is the
below-min_periods `Null(NaN)` and (mean) `count==0` never occurs — every emitted
value is finite. Drive the same `acc` left-fold (seeded -0.0 to match
`Iterator::sum`) off the raw `&[f64]` and emit a typed f64 column. Bit-identical:
same operands/order/values, all finite ⇒ `from_f64_values` all-valid.

## Proof
- `cargo test -p fp-frame expanding`: 48 passed, 0 failed.
- fp-bench expanding_sum 100k: **4.46 -> ~0.42 ms p50 = ~10.6x** — now faster than
  pandas (0.79 ms).
