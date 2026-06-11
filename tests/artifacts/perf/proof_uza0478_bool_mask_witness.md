# br-frankenpandas-uza04.79 proof - producer-carried bool-mask witness

## Target

- Bead: `br-frankenpandas-uza04.79`
- Context: upstream `br-frankenpandas-uza04.78` rejected an owned BooleanMask wrapper; this proof covers the deeper typed-storage selection descriptor follow-up while preserving that rejection record.
- Hotspot: `DataFrame::filter_rows(&Series)` with all-valid Bool masks generated from a trusted Series.
- Baseline profile: `loc_bool` repeatedly rescanned the same immutable every-other bool mask to rediscover an affine selection certificate.
- Lever: cache an exact `BoolAffineSelectionWitness { start, step, len }` on all-valid Bool column backing and consume it only on the same-index/no-duplicate `filter_rows` path.

## Implementation

- `ColumnData::Bool` now uses immutable `Arc<[bool]>`.
- `ScalarValues::LazyAllValidBool` stores `OnceLock<Option<BoolAffineSelectionWitness>>`.
- `Column::bool_affine_selection_witness()` scans once and returns a cached exact witness only for all-valid Bool columns.
- `DataFrame::filter_rows(&Series)` passes the trusted witness to a private `loc_bool_with_affine_witness`; public `loc_bool(&[bool])` keeps the old raw-slice behavior.
- Nullable, non-Bool, non-affine, wrong-length, duplicate-index, and non-identical-index cases continue through existing validation and fallback paths.

## Isomorphism proof

- Witness expansion is exactly `start + i * step` for every true mask position accepted by the builder.
- Frame length and affine bounds are still validated by `take_rows_by_affine_certificate_unchecked`; rejected witnesses fall back to the old scanner.
- Public raw-slice `loc_bool(&[bool])` receives no witness, preserving fallback/error behavior.
- Row order, index labels and names, column order, dtype/validity/null/NaN behavior, and f64 text/golden bits remain unchanged.
- Tie-breaking and RNG surfaces are not present for this filter workload.

## Golden outputs

Baseline and after SHA-256 values match exactly:

```text
filter_bool 1000   f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c
filter_bool 100000 2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea
```

Verification artifact: `tests/artifacts/perf/uza0478_after_golden_verify.txt`

## Benchmark

Command shape:

```text
high_ram_perf_baseline --profile uza0478-{base,after} --rows 100000 --iters 40 --warmup 5 --frame-cols 10
```

Workload: `filter_boolean_mask`

| Metric | Baseline | After | Delta |
| --- | ---: | ---: | ---: |
| mean_ms | 16.460499225 | 13.350708925 | 1.233x faster, 18.89% lower |
| p50_ms | 16.188327 | 13.142838 | 18.81% lower |
| p95_ms | 18.552456 | 15.444809 | 16.75% lower |
| p99_ms | 19.783078 | 16.461676 | 16.79% lower |
| throughput_rows_per_sec | 6075149.8866 | 7490238.9500 | 23.29% higher |
| checksum | 31251625000.0 | 31251625000.0 | unchanged |
| rows_out | 50000 | 50000 | unchanged |

Score: Impact 4 x Confidence 4 / Effort 3 = 5.33. Keep.

## Reprofile

- Deflated-fanout after-profile: `high_ram_perf_baseline --profile uza0478-after-profile-keycard100000 --rows 100000 --iters 20 --warmup 3 --frame-cols 10 --key-cardinality 100000`
- Samples: 4,474; lost samples: 0.
- `filter_boolean_mask`: mean `13.8317913 ms`, p50 `13.182878 ms`, p95 `16.938472 ms`, p99 `17.186441 ms`, checksum `31251625000.0`, rows_out `50000`.
- Suite residual shifted ahead of boolean filtering into CSV float formatting and groupby aggregation; `run_filter` is `7.34%` children and `DataFrame::filter_rows` / affine take are only tiny visible samples.

## Validation

- `rch exec -- cargo test -p fp-columnar bool_affine_selection_witness_is_exact_and_cached_uza0478 --lib -- --nocapture`
- `rch exec -- cargo test -p fp-frame dataframe_filter_rows_all_valid_bool_witness_preserves_surface_uza0478 --lib -- --nocapture`
- `rch exec -- cargo check -p fp-frame --lib`
- `rch exec -- cargo check -p fp-columnar --all-targets`
- `rch exec -- cargo clippy -p fp-frame --lib -- -D warnings`
- `rch exec -- cargo clippy -p fp-columnar --all-targets -- -D warnings`
- `git diff --check -- crates/fp-columnar/src/lib.rs crates/fp-frame/src/lib.rs .skill-loop-progress.md tests/artifacts/perf/uza0478_pass2_primitive_selection.md`

Notes:

- `cargo fmt -p fp-columnar -p fp-frame -- --check` still reports broad pre-existing rustfmt drift in `fp-frame` examples and unrelated old sections; touched hunks are whitespace-clean by `git diff --check`.
- `timeout 180s ubs crates/fp-columnar/src/lib.rs` completed with no critical findings and broad existing warnings. `timeout 240s ubs crates/fp-frame/src/lib.rs` timed out during the large-file Rust scan; no UBS process was left running.
