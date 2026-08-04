# uza04.105 — typed f64 ewm().mean() fast path — KEPT (~4.5x)

BlackThrush, 2026-06-13. Gap found via the fp-bench vs-pandas harness.

## Target
ewm_mean @ 100k float64: fp 4.74 ms vs pandas 0.57 ms = 8.33x SLOWER (biggest gap).
`Ewm::mean` materialized `.values()` (Scalar::Float64 per row) and emitted a
`Vec<Scalar>` + `Column::from_values` re-scan.

## Lever (one, bit-identical)
All-valid Float64 fast path: run the identical adjust=True `old_wt` recurrence
straight off the contiguous `&[f64]` and emit a typed f64 output via
`Column::from_f64_values`. `as_f64_slice` requires `validity.all()` and
`from_f64_values` marks NaN missing, so the slice is finite with no missing rows
— the Scalar path's `is_missing()||to_f64().is_nan()` branch never fires, every
output is `Float64(weighted_avg)` (finite), and `from_f64_values` (all-finite ⇒
all-valid) yields the identical column.

## Proof
- `cargo test -p fp-frame ewm`: 41 passed, 0 failed.
- Bit-identical by construction (same recurrence, finite-only, typed output).
- fp-bench ewm_mean 100k: **4.74 -> ~1.0 ms p50 (min 0.83) = ~4.5x** (8.33x ->
  ~1.8x vs pandas).

## Next swing
Same `.values()` + `Vec<Scalar>`/`from_values` pattern in ewm().sum(),
expanding().sum() (5.63x), rolling().mean()/std() (3.58x/5.06x) — typed
in+out gives the same class of win.
