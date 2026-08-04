# uza04.111 — loc typed f64 gather + FxHash label map — KEPT (~3.9x)

BlackThrush, 2026-06-13. Gap via fp-bench vs-pandas (indexing).

## Target
loc_labels @ 100k float64: fp 38.3 ms vs pandas 1.99 ms = 19.3x SLOWER.
`loc_with_columns` built a SipHash `positions_by_label` and, per column, did
`column.values()[pos].clone()` (Scalar materialization per column) + `Column::new`.

## Lever (one, bit-identical)
- `positions_by_label` -> FxHashMap.
- Per column: typed Float64 gather (`data[position]` straight from `as_f64_slice`,
  emit via `from_f64_values`). Every position indexes an existing row (loc fails
  closed on missing labels) and the slice is all-valid finite, so the gathered
  column is all-present finite Float64 == old `values()[pos].clone()`.

## Proof
- `cargo test -p fp-frame loc`: 91 passed.
- fp-bench loc_labels 100k: **38.3 -> ~8 ms p50 (min 7.4) = ~3.9x** (19.3x -> ~4x
  vs pandas).

## Next swing
Residual is the `positions_by_label` map building a `Vec<usize>` per label (100k
tiny allocs for a unique index). A unique-index single-position map (or an i64
index hashtable) is the next lever; shared with reindex's label resolution.
