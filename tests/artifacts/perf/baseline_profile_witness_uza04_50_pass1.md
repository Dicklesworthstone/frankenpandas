# br-frankenpandas-uza04.50 Pass 1 Baseline/Profile Witness

Timestamp: 2026-06-11T19:20:00Z
Owner: OrangePeak
Head: `59d8a011`

## Target

`br-frankenpandas-uza04.50` was opened for the post-`.49` `filter_bool`
residual originally described as `Column::new` / dtype-inference cost in the
affine filter output path.

The fresh profile split matters:

- Exact gate (`filter_bool 100000 1000`) reproduces the `Column::new`,
  `ColumnData::from_scalars`, and `infer_dtype` rows.
- The call stacks show those rows under `perf_profile::build_numeric_frame`,
  i.e. public `Column::from_values(Vec<Scalar::Float64>)` construction, not
  affine filter output column construction.
- Steady-state filter loop (`filter_bool 100000 20000`) is still dominated by
  `period2_boolean_mask_span` / `affine_boolean_mask_span` and affine frame
  assembly; no `Column::new` output path is hot there.

## Build

Command:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza04-50-pass1 \
RUSTFLAGS='-C force-frame-pointers=yes' \
rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Result: PASS.

RCH mode: local fail-open. The log starts with
`[RCH] local (no admissible workers: insufficient_slots=1,hard_preflight=8,active_project_exclusion=1)`.

## Golden SHA

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/golden_uza04_50_pass1_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/golden_uza04_50_pass1_filter_bool_100000.txt
```

`sha256sum -c tests/artifacts/perf/golden_uza04_50_pass1.sha256` passed.

## Baseline Timing

Command:

```text
hyperfine --warmup 3 --runs 10 \
  '/data/projects/.scratch/cargo-target-orangepeak-uza04-50-pass1/release-perf/examples/perf_profile filter_bool 100000 1000'
```

Result: `43.1 ms +/- 2.3 ms`, median `43.23038788 ms`,
range `40.38659888..47.01189188 ms`.

## Profiles

Exact gate profile:

```text
perf record -F 999 -g --call-graph dwarf \
  -o tests/artifacts/perf/perf_uza04_50_pass1_filter_bool_100000_1000.data \
  -- /data/projects/.scratch/cargo-target-orangepeak-uza04-50-pass1/release-perf/examples/perf_profile filter_bool 100000 1000
```

Captured 60 samples, 0 lost. Top no-children rows:

- `<fp_frame::DataFrame>::loc_bool`: 13.01% self.
- `<fp_columnar::Column>::new`: 10.41% self, from `Column::from_values` under `perf_profile::build_numeric_frame`.
- `<fp_columnar::ColumnData>::from_scalars`: 5.42% self, from `build_numeric_frame`.
- `fp_types::infer_dtype`: 2.86% self, from `build_numeric_frame`.

Steady-state filter-loop profile:

```text
perf record -F 999 -g --call-graph dwarf \
  -o tests/artifacts/perf/perf_uza04_50_pass1_filter_bool_100000_20000.data \
  -- /data/projects/.scratch/cargo-target-orangepeak-uza04-50-pass1/release-perf/examples/perf_profile filter_bool 100000 20000
```

Captured 188 samples, 0 lost. Top no-children rows:

- `<fp_frame::DataFrame>::loc_bool`: 45.58% self.
- `affine_boolean_mask_span` / `period2_boolean_mask_span`: dominant child rows.
- `<fp_frame::DataFrame>::new_with_axes`: 3.96% self.
- `BTreeMap<String, Column>::insert`: 2.95% self.
- `core::ptr::drop_in_place::<fp_columnar::Column>`: 4.32% self.

## Verdict

The `Column::new` / `infer_dtype` part of `.50` is profile-backed only as a
public homogeneous Float64 constructor cost exposed by the exact benchmark
gate. It is not in the affine filter output path. The valid pass-2 lever is
therefore `Column::from_values(Vec<Scalar::Float64>) -> Column::from_f64_values`
under a homogeneous Float64 proof. The steady-state period-2 verifier residual
must be routed separately after this pass rather than repeating `.49` or the
rejected block-verifier family.
