# uza04.108 — typed nullable f64 rolling var/std — KEPT (~5x)

BlackThrush, 2026-06-13. Gap via fp-bench vs-pandas harness.

## Target
rolling_std_w50 @ 100k float64: fp 7.79 ms vs pandas 1.54 ms = 5.06x SLOWER.
`rolling_var_online` (rolling var + std) materialized `.values()` input and boxed
`Vec<Scalar>` + `Column::from_values` output.

## Lever (one, bit-identical)
- Input: typed `as_f64_slice` view (NaN == missing), else materialize once.
- Output: convert each `state.output()` Scalar to (datum, valid-bit) and build via
  `from_f64_values_with_validity`. `Null(NullKind::NaN)` (nobs<min_periods) -> 0.0
  with bit CLEARED -> reads back `Null(NaN)`. Every `Float64(v)` (incl. the
  nobs<=ddof `Float64(NaN)` and the 0.0/variance cases) -> v with bit SET -> reads
  back `Float64(v)`. The 0.0-vs-NaN-for-cleared-bit distinction (LazyNullableFloat64
  reads cleared-bit NaN as `Float64(NaN)`) is why Null uses 0.0, not NaN.

## Proof
- `cargo test -p fp-frame rolling`: 82 passed, 0 failed.
- fp-bench rolling_std_w50 100k: **7.79 -> ~1.55 ms p50 (min 1.53) = ~5x** — now at
  pandas parity (1.54 ms).
