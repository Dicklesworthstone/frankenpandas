# br-frankenpandas-uza04.41 proof

## Target

- Scenario: `perf_profile filter_bool 100000`.
- Baseline head: `8a5a2b19d55d96c0952fe5a85fbc7c149b10077c`.
- Baseline profile: `filter_bool 100000 1000` = 0.764 ms/iter, with `Column::take_positions` at 70.29%.
- Rejected families avoided: fused mask scan and strided eager gather (`fp_rejected_filter_bool_fused_mask_fi6zx.txt`, `br-frankenpandas-uza04.2`).

## Lever

`fp-columnar` now shares all-valid Float64 dense buffers through `Arc<[f64]>` views and adds a lazy arithmetic-progression projection descriptor alongside the existing contiguous-slice view:

- source buffer: shared `Arc<[f64]>`
- projection: `start`, `step`, `len`
- materialization: `OnceLock<Vec<f64>>` for typed reads and `OnceLock<Vec<Scalar>>` for scalar reads

The fast path is gated to wide all-valid Float64 selections only. Nullable, non-Float64, non-regular, duplicate, and small gathers keep the previous paths.

## Isomorphism

- Ordering: the descriptor emits `source[start + i * step]` for `i = 0..len`, the same order as the validated `positions` list.
- Floating point: values are copied by `f64` value and verified by `to_bits()` in `float64_take_positions_regular_stride_defers_contiguous_gather`, preserving `-0.0` and infinities.
- Null/NaN: path is all-valid Float64 only; `Column::from_f64_values` keeps NaN out of all-valid slices, and nullable Float64 still uses the existing validity-gather path.
- Dtype/validity: output dtype remains `Float64`; validity is exactly `ValidityMask::all_valid(positions.len())`.
- Materialization: `values()`, serialization, equality, debug, and `as_f64_slice()` force the same contiguous row sequence as the old eager gather.
- Golden: `golden_after_uza04_41_filter_bool_1000.txt` is byte-identical to the pass-1 baseline `golden_post693_orangepeak_next_filter_bool_1000.txt`; `golden_after_uza04_41_filter_bool_100000.txt` forces the lazy 100k-row path and verifies with sha256.

## Benchmark

- Like-for-like wall clock: `filter_bool 100000 20`
  - before: 59.4 ms +/- 2.4 ms
  - after: 47.2 ms +/- 2.4 ms
  - delta: 1.26x faster
- Profile command: `filter_bool 100000 1000`
  - before: 0.764 ms/iter
  - after: 0.443 ms/iter
  - delta: 1.72x faster

Score: Impact 4 x Confidence 4 / Effort 2 = 8.0, keep.

## Artifacts

- `tests/artifacts/perf/build_after_uza04_41.txt`
- `tests/artifacts/perf/hyperfine_after_uza04_41_filter_bool_20.json`
- `tests/artifacts/perf/hyperfine_after_uza04_41_filter_bool_20.txt`
- `tests/artifacts/perf/hyperfine_after_uza04_41_filter_bool.json`
- `tests/artifacts/perf/hyperfine_after_uza04_41_filter_bool.txt`
- `tests/artifacts/perf/golden_after_uza04_41.sha256`
- `tests/artifacts/perf/golden_after_uza04_41.verify.txt`
- `tests/artifacts/perf/golden_after_uza04_41_filter_bool_1000.txt`
- `tests/artifacts/perf/golden_after_uza04_41_filter_bool_100000.txt`
- `tests/artifacts/perf/perf_after_uza04_41_filter_bool.data`
- `tests/artifacts/perf/perf_record_after_uza04_41_filter_bool.txt`
- `tests/artifacts/perf/perf_report_after_uza04_41_filter_bool.txt`

## Shifted Bottleneck

After-profile (`filter_bool 100000 1000`, 677 samples):

- `Column::take_positions`: 52.38% self / 52.42% children
- `DataFrame::loc_bool`: 27.45% self / 27.68% children
- `DataFrame::take_rows_by_positions_unchecked`: 10.94% self / 11.17% children

Next route: compute/share an arithmetic row-selection descriptor once in `DataFrame::loc_bool` and pass it through row take, so each column does not rescan the same positions list to rediscover the AP descriptor.
