# br-frankenpandas-uza04.50 proof - Float64 `Column::from_values` fast path

- Owner: OrangePeak
- Head before lever: `59d8a011`
- Target: profile-backed `filter_bool 100000 1000` constructor residual under numeric frame setup.
- Lever: when `Column::from_values` receives a non-empty all-`Scalar::Float64` vector, build through `Column::from_f64_values` and skip the generic `infer_dtype + Column::new + ColumnData::from_scalars` path. Mixed/null and empty inputs still use the old path.

## Baseline and profile

- Baseline command: `hyperfine --warmup 3 --runs 10 ... perf_profile filter_bool 100000 1000`
- Baseline mean: 43.1 ms +/- 2.3 ms, median 43.23038788 ms, range 40.38659888..47.01189188 ms.
- Exact 1000-iteration profile reproduced the targeted constructor rows:
  - `<fp_columnar::Column>::new`: 10.41% self
  - `<fp_columnar::ColumnData>::from_scalars`: 5.42% self
  - `fp_types::infer_dtype`: 2.86% self
- Steady 20000-iteration profile showed the next residual is not the constructor path; it is `DataFrame::loc_bool` plus affine/period-2 mask recognition and frame assembly.

## Isomorphism proof

- Output ordering: unchanged. The filter operation still uses the same mask and selection path; the lever only changes the homogeneous Float64 constructor used by setup.
- Index labels and column order: byte-for-byte identical golden dumps for `filter_bool` at `n=1000` and `n=100000`.
- Dtype: pure Float64 input still returns `DType::Float64`; mixed Float64/null and empty inputs keep the old fallback path.
- Validity/null/NaN: `from_f64_values` marks NaN invalid, matching the generic constructor for pure Float64 inputs. The focused test covers valid floats, `-0.0`, infinity, NaN payload bits, mixed null fallback, and empty fallback.
- Floating point: stored f64 payload bits are preserved for non-NaN values and for the NaN payload returned by `values()`.
- Tie-breaking and RNG: not applicable.

Golden SHA and byte equality:

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/golden_uza04_50_pass1_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/golden_uza04_50_pass1_filter_bool_100000.txt
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/golden_after_uza04_50_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/golden_after_uza04_50_filter_bool_100000.txt
```

Artifacts:

- `tests/artifacts/perf/golden_pair_uza04_50_raw.sha256`
- `tests/artifacts/perf/golden_pair_uza04_50_raw.verify.txt`
- `tests/artifacts/perf/golden_pair_uza04_50_filter_bool_1000.cmp.txt`
- `tests/artifacts/perf/golden_pair_uza04_50_filter_bool_100000.cmp.txt`

## Benchmark result

Paired order:

- Before: 42.8 ms +/- 3.3 ms, median 41.98208064 ms.
- After: 21.8 ms +/- 1.6 ms, median 21.86309664 ms.
- Result: after ran 1.96 +/- 0.21 times faster.

Reversed order:

- After: 22.9 ms +/- 2.7 ms, median 22.29817712 ms.
- Before: 44.9 ms +/- 4.5 ms, median 44.48081712 ms.
- Result: after ran 1.96 +/- 0.31 times faster.

Score gate:

- Impact: 4
- Confidence: 4
- Effort: 2
- Score: 8.0
- Verdict: keep.

## Validation

- `rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`: pass on `vmi1227854`.
- `rch exec -- cargo test -p fp-columnar from_values_float64_fast_path_preserves_surface --lib`: pass on `vmi1227854`.
- `rch exec -- cargo check -p fp-columnar --all-targets`: pass on `vmi1227854`.
- `rch exec -- cargo clippy -p fp-columnar --all-targets -- -D warnings`: pass on `vmi1227854`.
- `cargo fmt -p fp-columnar --check`: pass.
- `git diff --check -- crates/fp-columnar/src/lib.rs ...`: pass.
- `ubs crates/fp-columnar/src/lib.rs`: exit 0, Critical 0. Existing warnings remain scanner inventory, not introduced by this lever.

## Re-profile result

Post-change `filter_bool 100000 20000` profile:

- `<fp_frame::DataFrame>::loc_bool`: 49.86% self.
- `<fp_frame::DataFrame>::new_with_axes`: 7.29% self.
- `affine_boolean_mask_span` / `period2_boolean_mask_span` remain the dominant child rows.
- The old constructor target rows (`Column::new`, `ColumnData::from_scalars`, `infer_dtype`) are absent from the steady post-change report.

Next target: route follow-up work to the residual affine/period-2 mask verifier and frame assembly path rather than more constructor tuning.
