# br-frankenpandas-uza04.121 rejection proof: df_dot output NaN witness

## Target

- Hotspot: `DataFrame::dot` for `df_dot 100000 6` / `df_dot 100000 20`.
- Prior route: `.120` proved lazy shared output storage preserved goldens but regressed badly, so this pass kept contiguous output columns.
- Candidate primitive: carry exact per-output-column NaN witnesses out of the GEMM store loop and use a hidden all-valid Float64 constructor when a column has no NaN, avoiding `Column::from_f64_values`' redundant NaN scan.

## Candidate Isomorphism

- Ordering: unchanged. Row-band sorting and column assembly order were unchanged.
- Floating point: unchanged. Every `C[i][j]` used the same `l = 0..k` ascending f64 fold and the same products/additions.
- Missing semantics: preserved by construction. If any stored output value for a column was NaN, the candidate used the existing `Column::from_f64_values` path for that column. Only columns with no observed NaN used the all-valid constructor.
- Tie-breaking/RNG: not applicable.
- Golden output:
  - Base `df_dot 2000`: `ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535`
  - After `df_dot 2000`: `ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535`
  - Base `df_dot 5000`: `04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`
  - After `df_dot 5000`: `04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`
  - `cmp -s` passed for both baseline-vs-after golden files.

## Benchmark Gate

- Baseline-only artifact: `lavender_df_dot_nan_witness_base_hyperfine_100000x6.txt`
  - baseline: `958.9 ms +/- 28.0 ms`
- Paired `100000x6`:
  - forward: baseline `978.3 ms +/- 31.5 ms`; candidate `996.6 ms +/- 28.0 ms` (baseline `1.02x` faster)
  - reversed: candidate `973.8 ms +/- 26.4 ms`; baseline `1.108 s +/- 0.068 s` (candidate `1.14x` faster, outlier warning)
- Longer paired `100000x20`:
  - forward: baseline `2.620 s +/- 0.034 s`; candidate `2.562 s +/- 0.041 s` (candidate `1.02x` faster)
  - reversed: candidate `2.612 s +/- 0.101 s`; baseline `2.662 s +/- 0.106 s` (candidate `1.02x` faster)
- Verdict: reject. The longer run suggests only a ~2% improvement, below the Score>=2.0 keep bar for this campaign and too small for the extra branch/witness state in the GEMM store loop.

## Validation

- `rch exec -- cargo check -p fp-frame --lib`: passed after mechanical tuple/attribute fixes.
- Candidate release-perf builds fell open locally under the RCH wrapper because no workers were admissible at dispatch time; this limitation is recorded in the build artifacts.
- No source code from the candidate was retained.

## Next Route

Do not repeat output-NaN-witness or lazy-output-storage micro-levers. The next `df_dot` pass needs a larger structural primitive:

- a column-panel microkernel that emits final contiguous column chunks without replaying the A matrix excessively,
- a row-band representation that keeps contiguous typed slices available without an Arc-slice copy,
- or a deeper blocked/recursive GEMM kernel that reduces the output-boundary cost rather than shaving the existing assembly scan.
