# uza04.113 — loc unit-range arithmetic resolution — KEPT (~2.9x, near pandas parity)

BlackThrush, 2026-06-13. fp-bench vs-pandas (indexing).

## Target
After uza04.111 (typed gather), loc_labels 100k float64 sat at ~8 ms vs pandas
1.99 ms. The bottleneck was NOT the gather but `positions_by_label`: a
`FxHashMap<&IndexLabel, Vec<usize>>` allocating a `Vec<usize>` per index label
(~100k tiny allocs for a unique index) + per-requested-label probe. (A parallel
per-column gather was tried first and was neutral/regressive — confirming the
gather isn't the bottleneck — and reverted.)

## Lever (one, bit-identical)
When the index is an Int64 unit-range (pandas RangeIndex default), resolve each
requested Int64(v) to position v-start arithmetically — O(1) per label, no map,
no Vec allocs. The unit range is strictly unique, so each label matches exactly
one position (or none ⇒ fail closed, same message as the map miss); the emitted
out_label `index_labels[p]` equals the requested `Int64(v)`.

## Proof
- `cargo test -p fp-frame loc`: 91 passed.
- fp-bench loc_labels 100k: **~8 -> ~2.75 ms p50 (min 2.6) = ~2.9x** (~1.4x vs
  pandas 1.99ms, near parity). Total from original 38.3ms: ~14x.
