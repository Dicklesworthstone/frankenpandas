# 8sxez (completion) — value_counts_with_options Float64 numeric index — PARITY FIX

BlackThrush, 2026-06-13. Confirmed against the live pandas 2.2.3 oracle.

## Context

The basic `Series::value_counts()` path was already converted to emit a numeric
`Float64Index` for a float Series (peer work on bead 8sxez, via
`scalar_to_series_value_counts_index_label` inside `materialize_value_counts_output`).
But the **with-options** path — `value_counts_with_options(normalize, sort,
ascending, dropna)`, i.e. `pd.Series.value_counts(...)` with any kwargs — was
left routing through the Utf8 mapper, so a float Series still got a stringified
**object** index there. That is a parity bug: pandas returns float64 in every case.

```
pd.Series([1.0,2.0,2.0,3.5]).value_counts(normalize=True).index.dtype  -> float64
                                          (dropna=False / sort=False / ascending=True) -> float64
```

## Lever (one): add the Float64 arm to the with-options label match

`value_counts_with_options`'s label loop already had an inline match to keep
distinct null kinds (None/NaN/NaT) for `dropna=False` (br-frankenpandas-joeff).
Added one arm — `Scalar::Float64(v) => IndexLabel::Float64(OrderedF64(v))` —
keeping the null arm and the `other => scalar_to_value_counts_index_label`
fallback unchanged, so pivot/crosstab/mode/describe (shared-mapper callers) are
untouched.

## Proof (verified bit-exact vs live pandas 2.2.3)

Series `[1.0, 2.0, 2.0, 3.5, 3.5, 3.5]` (example `vc_float_golden`):

```
                FP                                pandas 2.2.3
normalize     3.5=0.5 2.0=0.333… 1.0=0.166…      3.5=0.5 2.0=0.333… 1.0=0.166…   float64
dropna_false  3.5=3 2.0=2 1.0=1                  3.5=3 2.0=2 1.0=1               float64
sort_false    1.0=1 2.0=2 3.5=3                  1.0=1 2.0=2 3.5=3               float64
ascending     1.0=1 2.0=2 3.5=3                  1.0=1 2.0=2 3.5=3               float64
```

Labels are now `IndexLabel::Float64(OrderedF64(..))` (was `Utf8("3.5")`); values,
order, and dtype all match pandas.

- `cargo test -p fp-frame --lib value_counts`: 33 passed, 0 failed. One test
  (`series_value_counts_null_bucket_at_first_seen_position_joeff`) updated: it had
  asserted `Utf8("1.0")`/`"2.0"`/`"5.0"`/`"3.0"` while its own comment documented
  pandas returning floats — corrected to `Float64`.
- Full `cargo test -p fp-frame --lib` + `cargo test -p fp-conformance value_counts`: green.

Parity correctness fix (behavior parity is absolute); also removes the per-distinct
`format!`/String alloc for the float with-options path. Completes 8sxez across both
Series value_counts entry points.
