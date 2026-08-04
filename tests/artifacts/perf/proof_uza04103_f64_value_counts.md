# uza04.103 — Float64 value_counts bit-tally fast path — KEPT (1.74x)

BlackThrush, 2026-06-13. Gap found via the restored fp-bench vs-pandas harness.

## Target (fp-bench vs pandas 2.2.3, value_counts @ 100k float64, all-unique)

```
BEFORE  fp p50 = 21.69 ms   pandas p50 = 5.37 ms   -> 4.04x SLOWER (biggest dataframe_ops gap)
```

Root cause: `Series::value_counts` had int64-direct and utf8-contiguous fast
paths but NO float64 path — float columns fell to the generic
`HashMap<ScalarKey>` tally: a `Scalar::Float64` materialized per row, wrapped in
`ScalarKey`, hashed through the std HashMap's SipHash.

## Lever (one, bit-identical)

All-valid Float64 bit-tally: `FxHashMap<u64, usize>` keyed on the canonicalized
bit pattern (`if v == 0.0 { 0 } else { v.to_bits() }`), presized to `len`.
- Key canonicalization is identical to `scalar_key_allow_missing`:
  `normalized = if v==0.0 {0.0} else {v}; FloatBits(normalized.to_bits())`
  (`0.0f64.to_bits() == 0`), so `-0.0`/`+0.0` merge exactly as before.
- `as_f64_slice` requires `validity.all()` (NaN is marked missing, so any-NaN
  columns return None and keep the generic path) — nothing to skip; a defensive
  `v.is_nan()` continue matches `scalar_key_skip_missing`.
- First-seen row order, per-distinct `Scalar::Float64(v)`, and the stable
  descending-by-count sort are unchanged.

## Proof

- `cargo test -p fp-frame value_counts`: 32 passed, 0 failed.
- Bit-identical by construction (same key, first-seen order, stored value, sort).
- fp-bench value_counts @ 100k float64 (min-of-25, 3 runs):
  **21.69 -> 12.5 ms = 1.74x** (min ~12.3 ms).

## Residual / next swing

12.5 ms vs pandas 5.4 ms (2.3x) is now dominated by the 100k Utf8 index-label
construction (`format!("{v:?}")` per distinct value) + the stable sort. The label
format is golden-locked (current value_counts emits a Utf8 index). Closing it
requires emitting a `Float64` IndexLabel output (which is also what pandas
returns — a float index), a parity-gated change needing oracle sign-off + golden
regen, not a bit-identical lever. Target there: ~5-6 ms (pandas parity).
