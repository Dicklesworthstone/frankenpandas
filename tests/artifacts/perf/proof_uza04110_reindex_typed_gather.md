# uza04.110 — reindex precompute positions + typed f64 gather — KEPT (~8x)

BlackThrush, 2026-06-13. Gap via fp-bench vs-pandas harness (indexing category,
newly added).

## Target
reindex @ 100k x10 float64: fp 65.2 ms vs pandas 1.11 ms = 58.6x SLOWER (biggest
gap found). DataFrame::reindex re-resolved every new label INSIDE the per-column
loop (cols × new_len SipHash lookups) AND called `col.values()` per column
(materializing the whole source as `Vec<Scalar>` 10×) + `Column::new` re-scan.

## Lever (one, bit-identical)
- Build the old-label→row FxHashMap once; resolve each new label to
  `Some(row)`/`None` ONCE (shared across columns).
- Per column: typed Float64 gather (present → `data[idx]` + validity bit SET;
  missing → 0.0 + bit CLEARED) via `from_f64_values_with_validity` —
  `Float64(data[idx])` == old `values()[idx].clone()`, cleared-bit-0.0 reads back
  `Null(NullKind::NaN)` == old fill, dtype stays Float64. Non-f64 columns keep
  the Scalar path but reuse the precomputed positions.

## Proof
- Golden sha256 BYTE-IDENTICAL: reindex_str n=2000/5000 (Utf8 cols → Scalar path).
- `cargo test -p fp-frame reindex`: 28 passed.
- fp-bench reindex 100k: **65.2 -> ~8 ms p50 (min 6.5) = ~8x** (58.6x -> ~7x vs
  pandas).

## Next swing
Residual ~8ms is the 100k IndexLabel FxHash lookups + scattered gather; pandas
uses the index's prebuilt hashtable (1.1ms). A direct-i64 index hashtable (or
the cached unit-range lookup) is the next lever. loc_labels (19x) shares the
same label-resolution path.
