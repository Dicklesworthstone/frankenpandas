# br-frankenpandas-uza04.80 current-main proof - Float64 `Column::from_values` fast path

- Owner: OrangePeak
- Base: `origin/main` at `98ed64a9`
- Integrated keep bead: `br-frankenpandas-uza04.82`
- Follow-up bead: `br-frankenpandas-uza04.83`
- Historical artifact name: `uza04_50`, from the stale local branch whose numeric bead IDs collided with current `origin/main`.

## Lever

`Column::from_values` now detects non-empty homogeneous `Scalar::Float64` input and builds with `Column::from_f64_values`, skipping the generic `infer_dtype + Column::new + ColumnData::from_scalars` path. Empty input and mixed Float64/null input preserve the previous fallback path.

## Current-main golden proof

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/golden_current_before_uza04_80_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/golden_current_before_uza04_80_filter_bool_100000.txt
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/golden_current_after_uza04_80_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/golden_current_after_uza04_80_filter_bool_100000.txt
```

Both current-main before/after golden dumps compare byte-for-byte equal with `cmp`.

## Current-main benchmark

Paired order:

- Before current main: 48.4 ms +/- 5.5 ms, median 48.12036062 ms.
- After Float64 fast path: 23.9 ms +/- 2.3 ms, median 23.73208912 ms.
- Result: after ran 2.03 +/- 0.30 times faster.

Reversed order:

- After Float64 fast path: 22.0 ms +/- 1.8 ms, median 21.75002202 ms.
- Before current main: 47.3 ms +/- 2.6 ms, median 47.00853152 ms.
- Result: after ran 2.15 +/- 0.21 times faster.

Score gate:

- Impact: 4
- Confidence: 4
- Effort: 2
- Score: 8.0
- Verdict: keep.

## Current-main validation

- `rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`: pass on `vmi1227854` for before and after binaries.
- `rch exec -- cargo test -p fp-columnar from_values_float64_fast_path_preserves_surface --lib`: pass on `vmi1227854`.
- `rch exec -- cargo check -p fp-columnar --all-targets`: pass on `vmi1227854`.
- `rch exec -- cargo clippy -p fp-columnar --all-targets -- -D warnings`: pass on `vmi1227854`.
- `cargo fmt -p fp-columnar --check`: pass.
- `git diff --check --cached -- crates/fp-columnar/src/lib.rs .beads/issues.jsonl`: pass.
- `ubs crates/fp-columnar/src/lib.rs`: exit 0, Critical 0.

Full generated perf/UBS logs preserve tool whitespace; the source/metadata whitespace check is scoped above.
