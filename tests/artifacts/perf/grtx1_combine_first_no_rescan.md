# br-frankenpandas-grtx1 — Series.combine_first no-rescan Float64 output

## Lever

`Series::combine_first` already had a typed identity fast path for identical,
duplicate-free Float64 indexes:

```text
out[i] = self_valid(i) && !self[i].is_nan() ? self[i] : other[i]
```

For this branch `other.as_f64_slice()` proves an all-valid Float64 backing, and
the self-present predicate rejects NaN. Therefore the selected output is
NaN-free. `grtx1` uses `Column::from_f64_values_all_valid_unchecked(out)` for
that branch instead of rescanning the 2M output buffer in
`Column::from_f64_values(out)`.

## Behavior proof

- Duplicate indexes still take the duplicate-aware alignment path before the
  fast path.
- Identical non-duplicate indexes preserve original order, matching the existing
  pandas contract.
- Missingness is unchanged: `sv.get(i) && !sd[i].is_nan()` is exactly the
  Float64 `!is_missing` predicate for the self side; otherwise the other value
  is selected.
- The all-valid stamp is only used when `other.as_f64_slice()` succeeds, so the
  fill source cannot contribute a missing/NaN slot under the column contract.

Focused rch test:

```text
CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b \
  rch exec -- cargo test -p fp-frame \
  combine_first_float64_identity_fast_path_keeps_all_valid_backing_grtx1 \
  --release -- --nocapture

result: 1 passed, 0 failed on hz2
```

## Perf evidence

Workload: `bench_combine_cc`, 2M Float64 same index, self has ~50% NaN, other
fills, best of 30.

Same-worker FrankenPandas A/B on rch `hz2`:

| build | best |
|---|---:|
| baseline `05eb4666` | 19.375061 ms |
| grtx1 run 1 | 18.331997 ms |
| grtx1 run 2 | 18.399917 ms |

Decision: keep. The repeated FP-side gain is ~1.05x, modest but measured and
explained by removing one O(n) scan.

Local head-to-head after the change, using the rch-retrieved release example:

| engine | best |
|---|---:|
| pandas 2.2.3 | 7.346722 ms |
| FrankenPandas grtx1 | 17.938418 ms |

Ratio vs pandas: 0.41x. This remains a loss; next work must attack output
allocation/packed select, not another no-rescan micro-lever.
