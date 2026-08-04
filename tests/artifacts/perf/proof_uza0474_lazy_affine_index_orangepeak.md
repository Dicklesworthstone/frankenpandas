# br-frankenpandas-uza04.74 proof - lazy affine/strided Int64 index labels

## Target

- Bead: `br-frankenpandas-uza04.74`
- Lever: replace eager Int64 index-label vectors in `DataFrame::loc_bool` affine filtering with lazy affine/strided Int64 index backing.
- Head before edit: `328d6228016d788d615c52ec1352666250f5fead`
- Final commit parent after clean rebase: `c47efa6f9899f54d7222052e0e8dc2cb43ac9b85`
- Scope: `crates/fp-index/src/lib.rs`, `crates/fp-frame/src/lib.rs`

## Baseline and profile

- Build: `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0474-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- RCH result: failed open locally (`no admissible workers: insufficient_slots=1,hard_preflight=1`); crate-scoped build passed.
- Baseline hyperfine: `filter_bool 100000 1000` = `152.8 ms +/- 3.3 ms`, range `147.2..158.9 ms`, 10 runs.
- Baseline profile: `95.25%` self in `<fp_frame::DataFrame>::loc_bool`; annotation showed the accepted affine path still pushing materialized Int64 labels at `labels.push(values[pos])`.
- Baseline artifacts: `tests/artifacts/perf/uza0474_pass1_20260611T010909Z_*`

## Implementation

- Added `Int64AffineLabels` and `Int64StridedLabels` lazy backing to `IndexLabels`.
- Added `Index::new_known_unique_int64_affine_range` for arithmetic Int64 labels.
- Added `Index::from_i64_strided_values` for typed-source strided labels.
- Replaced eager loops in `take_rows_by_affine_certificate_unchecked`:
  - Unit-range source labels now produce lazy affine labels for non-unit filter steps.
  - Cached/materializable typed Int64 source labels now produce lazy strided labels.
  - Non-typed labels still use the old materialized fallback.

## Isomorphism

- Row order: unchanged; the same affine certificate `(start, step, len)` defines output row order.
- Index label values: unchanged; lazy affine and strided backings materialize exactly the old eager sequence.
- Duplicate semantics: unchanged; arbitrary strided typed-source labels do not pre-seed duplicate caches.
- Sort semantics: unchanged; only empty/singleton/positive-step arithmetic affine labels pre-seed ascending sort.
- Column order and names: unchanged; `DataFrame::new_with_axes` receives the same column map/order and logical index.
- Dtypes, validity, nulls, NaNs, and f64 bits: unchanged; column gather path is untouched.
- Tie-breaking: not applicable to boolean filtering.
- RNG: not used.
- Fallback: non-affine masks and non-typed/non-range labels still use existing materialized paths.
- Overflow boundary: singleton `i64::MAX` affine labels are tested to avoid post-last checked-add overflow.

## Golden proof

Baseline SHA-256:

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/uza0474_pass1_20260611T010909Z_golden_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/uza0474_pass1_20260611T010909Z_golden_filter_bool_100000.txt
```

After SHA-256:

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/uza0474_after_20260611T013700Z_golden_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/uza0474_after_20260611T013700Z_golden_filter_bool_100000.txt
```

Hash-column diff was empty.

## Performance

Build after:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0474-after RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

RCH result: failed open locally; crate-scoped release-perf build passed.

Paired hyperfine:

```text
baseline: 145.8 ms +/- 6.7 ms
after:     95.2 ms +/- 5.6 ms
summary:  after ran 1.53x +/- 0.11 faster
```

Reversed paired hyperfine:

```text
after:     99.3 ms +/- 5.2 ms
baseline: 145.5 ms +/- 7.2 ms
summary:  after ran 1.46x +/- 0.11 faster
```

Score: Impact 4 x Confidence 4 / Effort 2 = 8.0, keep.

## Validation

Passed:

```text
rch exec -- cargo test -p fp-index int64_affine_range_index_preserves_materialized_surface --lib
rch exec -- cargo test -p fp-index int64_strided_index_preserves_duplicate_and_unsorted_semantics --lib
rch exec -- cargo test -p fp-frame dataframe_loc_bool_affine_index_labels_materialize_correctly --lib
rch exec -- cargo test -p fp-frame dataframe_loc_bool --lib
rch exec -- cargo check -p fp-index --all-targets
rch exec -- cargo check -p fp-frame --lib
rch exec -- cargo clippy -p fp-index --all-targets -- -D warnings
rch exec -- cargo clippy -p fp-frame --lib -- -D warnings
git diff --check -- crates/fp-index/src/lib.rs crates/fp-frame/src/lib.rs .skill-loop-progress.md
```

Rerun after rebasing onto `origin/main` `c47efa6f9899f54d7222052e0e8dc2cb43ac9b85`:

```text
rch exec -- cargo test -p fp-index int64_affine_range_index_preserves_materialized_surface --lib
rch exec -- cargo test -p fp-index int64_strided_index_preserves_duplicate_and_unsorted_semantics --lib
rch exec -- cargo test -p fp-frame dataframe_loc_bool --lib
```

Rustfmt caveat:

```text
cargo fmt -p fp-index -- --check
cargo fmt -p fp-frame -- --check
```

Both still fail on broad pre-existing formatting drift in examples and unrelated old sections. Touched hunks were manually cleaned and `git diff --check` is clean.

UBS caveat:

```text
ubs crates/fp-index/src/lib.rs crates/fp-frame/src/lib.rs
```

The run was attempted before commit but became stuck for several minutes in the Rust module while scanning the 100k-line `fp-frame` file and emitted no findings. The exact UBS PIDs from that run were terminated so no scanner session remained active.

## After profile

- `perf_profile filter_bool 100000 20000`: `0.057 ms/iter`, `sink=1000000000`.
- `perf report`: `<fp_frame::DataFrame>::loc_bool` remains the envelope symbol at `92.32%`.
- Annotation shows the old eager typed-label line replaced by `Index::new_known_unique_int64_affine_range` and `Index::from_i64_strided_values`.
- Shifted residual: output-frame reconstruction and remaining fast-path assembly (`new_with_axes`, map insertion, memcmp/memmove), not eager Int64 label materialization.
- Follow-up filed: `br-frankenpandas-uza04.75`.

## Artifacts

- `tests/artifacts/perf/uza0474_pass1_20260611T010909Z_*`
- `tests/artifacts/perf/uza0474_pass2_20260611T011834Z_primitive_selection.md`
- `tests/artifacts/perf/uza0474_after_20260611T013700Z_*`
