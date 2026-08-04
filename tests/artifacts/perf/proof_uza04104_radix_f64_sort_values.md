# uza04.104 — radix f64 sort_values ascending — KEPT (~2.8x, closes vs-pandas gap)

BlackThrush, 2026-06-13. Gap found via the restored fp-bench vs-pandas harness.

## Target
sort_values_single @ 100k float64: fp 13.6 ms vs pandas 5.37 ms = 2.5x SLOWER.
`typed_dense_sort_order` sorted f64 via an O(n log n) `partial_cmp` comparison sort.

## Lever (one, bit-identical for ascending NaN-free)
Route ascending all-valid-f64 sort through the existing stable O(n)
`fp_columnar::radix_argsort_f64` (already golden-verified via spearman ranking).
Stable (equal values keep input order, like the stable `sort_by`) + numeric
ascending over all-finite f64 = the same permutation. Any NaN bails to the
Scalar path (unchanged). Descending keeps the comparison sort (its
`order.reverse()` preserves equal-values-in-input-order, which a reversed radix
would not).

## Proof
- Golden sha256 BYTE-IDENTICAL: sort_single n=2000/5000 unchanged.
- `cargo test -p fp-frame sort_values`: 26 passed.
- fp-bench sort_values_single 100k float64: **13.6 -> ~4.8 ms p50 (min 3.7) = ~2.8x**;
  now FASTER than pandas (5.37 ms) — gap closed.
