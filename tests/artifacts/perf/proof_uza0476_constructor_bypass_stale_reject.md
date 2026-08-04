# br-frankenpandas-uza04.76 - affine filter constructor-bypass stale rejection

Status: closed as prior rejected/stale. No runtime source changed.

## Bead Target

`br-frankenpandas-uza04.76` targeted bypassing `DataFrame::new_with_axes`
constructor normalization for affine boolean filter outputs after the
every-other mask certificate keep.

The bead already contains a rejection note for that exact lever:

- `from_prevalidated_axes` / constructor bypass preserved goldens.
- Paired release-perf hyperfine regressed in both 20k and 60k iteration gates.
- The apparent `new_with_axes` profile share was a large share of a tiny
  absolute runtime, not a useful wall-clock target.

## Current Evidence

Current binary:

`/data/projects/.scratch/cargo-target-lavenderstone-vhrv5-base/release-perf/examples/perf_profile`

This binary was RCH-built on worker `vmi1153651` from current `main` for the
same session.

Golden-output SHA256:

- `filter_bool 1000`: `f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c`
- `filter_bool 100000`: `2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea`

Current hyperfine:

- `filter_bool 100000x20000`: `69.9 ms +/- 4.7 ms`
- `filter_bool 100000x60000`: `169.6 ms +/- 5.1 ms`

Artifacts:

- `tests/artifacts/perf/uza0476_baseline_filter_bool_100000_20000.json`
- `tests/artifacts/perf/uza0476_baseline_filter_bool_100000_60000.json`

## Decision

Close as stale/prior rejected. The exact constructor-bypass lever described by
this bead was already measured and rejected, and current timings are in the
single-digit microsecond-per-call range. Repeating this bead would violate the
non-repeat boundary around constructor normalization bypasses.

No `fp-frame` or harness source was edited.

## Next Route

Move to `br-frankenpandas-uza04.51` to verify whether the broader affine /
period-2 filter residual has any current profiler-evident hotspot outside the
constructor-bypass family. If not, route out of filter_bool rather than repeat
the exhausted constructor-normalization lever.
