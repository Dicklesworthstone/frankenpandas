# uza04.107 — typed nullable f64 rolling sum/mean output — KEPT (~7.9x)

BlackThrush, 2026-06-13. Gap via fp-bench vs-pandas harness.

## Target
rolling_mean_w10 @ 100k float64: fp 6.33 ms vs pandas 1.77 ms = 3.58x SLOWER.
`rolling_online_sum_mean` already used a typed f64 INPUT (as_f64_slice) but boxed
the output into `Vec<Scalar>` + `Column::from_values` (dtype/validity re-scan).

## Lever (one, bit-identical, any min_periods)
Emit raw f64 + a validity bitset and build via `from_f64_values_with_validity`.
Below-min_periods slots store **0.0 (NOT NaN)** with the validity bit CLEARED:
`LazyNullableFloat64` reads a cleared-bit, non-NaN datum back as
`Null(NullKind::NaN)` — identical to the boxed `Scalar::Null(NullKind::NaN)`.
(A first attempt stored NaN there; `LazyNullableFloat64` reads cleared-bit NaN
back as `Float64(NaN)`, which broke a groupby-rolling test — fixed by storing
0.0.) Present results are finite (online sum skips NaN inputs) with the bit set
⇒ `Float64(r)`.

## Proof
- `cargo test -p fp-frame rolling`: 82 passed, 0 failed (incl. the
  groupby-rolling reductions test that caught the NaN-vs-0.0 read-back).
- fp-bench rolling_mean_w10 100k: **6.33 -> ~0.8 ms p50 (min 0.78) = ~7.9x** —
  now faster than pandas (1.77 ms).

## Next swing
rolling_std/var use a separate `rolling_var_online` (still Vec<Scalar> output);
rolling_std_w50 stays ~5x slower. Same typed-nullable-output lever applies.
