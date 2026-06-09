# br-frankenpandas-uza04.43 rejection proof

## Candidate

Lazy positive-step Int64 arithmetic range labels for row takes over lazy Int64
range indexes. Regular `filter_bool` masks over the default index can represent
output labels such as `0,2,4,...` as `(start=0, step=2, len=n/2)` instead of
gathering a `Vec<i64>`.

## Profile-backed target

Current `origin/main` baseline profile for `perf_profile filter_bool 100000 1000`:

- `Column::take_positions`: 58.43% children / 57.80% self
- `DataFrame::loc_bool`: 26.93% children / 26.02% self
- `DataFrame::take_rows_by_positions_unchecked`: 8.37% children / 8.23% self

The candidate attacked only the frame/index materialization side of this path.

## Behavior proof

- Ordering preserved: yes. The output index sequence is `source_start + source_step * position[i]` for the same `positions` slice, and the fallback path is unchanged for non-arithmetic positions.
- Tie-breaking unchanged: N/A. Boolean row filtering does not introduce comparisons or ties.
- Floating-point unchanged: yes. The candidate does not touch column value selection or Float64 arithmetic; the focused frame test checked `to_bits()` for `1.25` and `inf`.
- RNG unchanged: N/A.
- Golden outputs:
  - `filter_bool 1000`: `f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c`
  - `filter_bool 100000`: `2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea`
  - `golden_compare_uza04_43_current.txt` records before/after byte identity.

Focused tests passed:

- `cargo test -p fp-index known_unique_int64_arithmetic_range_materializes_and_caches --lib`
- `cargo test -p fp-frame dataframe_loc_bool_regular_default_index_uses_arithmetic_labels --lib`

## Benchmark decision

Paired hyperfine against current `origin/main`:

- `filter_bool 100000 20`: 50.3 ms +/- 2.3 before vs 48.1 ms +/- 2.2 after, 1.05x +/- 0.07.
- `filter_bool 100000 1000`: 489.9 ms +/- 38.0 before vs 471.0 ms +/- 12.1 after, 1.04x +/- 0.08.

Direct repeated 1000-iteration runs were neutral to slightly negative:

- Before: 0.446-0.460 ms/iter.
- After: 0.453-0.473 ms/iter.

Score: 0.0. The candidate preserved behavior but did not prove a robust real
win, so source hunks are not retained.

## Next route

Do not retry descriptor rediscovery, unchecked-position, fused-mask,
eager-stride, or index-label-only variants. The deeper primitive is the
graveyard `Vectorized Execution + Morsel-Driven Parallelism` family: selection
vectors should remain a compact operator payload through downstream operators
instead of forcing early tuple/index materialization.

Operational next bead from the ready queue:

- `br-frankenpandas-jbyuc.1.1.1.1.1.1.1.1`: re-profile join-only after cached
  parallelism so benchmark input construction is separated from the
  pandas-observable merge path before the next one-lever optimization.
