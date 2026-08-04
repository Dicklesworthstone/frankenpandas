# br-frankenpandas-uza04.76 proof - rejected prevalidated constructor bypass

## Target

- Bead: `br-frankenpandas-uza04.76`
- Candidate lever: use an internal prevalidated constructor in the affine boolean filter fast path to bypass `DataFrame::new_with_axes` column-order normalization.
- Scope attempted: `crates/fp-frame/src/lib.rs`
- Final source verdict: rejected; source hunk removed before commit.

## Baseline and profile

- Build: `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0476-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- Baseline hyperfine: `filter_bool 100000 1000` = `48.5 ms +/- 3.2 ms`, 12 runs.
- Baseline profile: `perf_profile filter_bool 100000 20000` = `0.008 ms/iter`, `sink=1000000000`.
- Baseline residual: `<fp_frame::DataFrame>::loc_bool` remained the envelope; `new_with_axes` appeared under it, but the sampled share was not large enough to move paired hyperfine by itself.

## Candidate

- Added a private prevalidated constructor with debug-only invariants.
- Routed only `take_rows_by_affine_certificate_unchecked` through it.
- Added a focused metadata test for non-default column order and column MultiIndex.
- After paired measurement failed, removed both the constructor bypass and the test addition.

## Golden proof

Baseline SHA-256:

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/uza0476_base_golden_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/uza0476_base_golden_filter_bool_100000.txt
```

Candidate SHA-256:

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/uza0476_after_golden_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/uza0476_after_golden_filter_bool_100000.txt
```

Hash-only diff: empty.

## Performance

Paired hyperfine:

```text
baseline:  46.9 ms +/- 3.6 ms
candidate: 47.2 ms +/- 3.6 ms
summary: baseline ran 1.01x +/- 0.11 faster
```

Reversed paired hyperfine:

```text
candidate: 46.9 ms +/- 3.6 ms
baseline:  50.2 ms +/- 3.9 ms
summary: candidate ran 1.07x +/- 0.12 faster
```

Score: Impact 1 x Confidence 2 / Effort 2 = 1.0, reject.

## Validation

Candidate-era focused test passed:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0476-check rch exec -- cargo test -p fp-frame dataframe_loc_bool_affine_index_labels_materialize_correctly --lib -- --nocapture
```

No source validation is needed for the final rejection commit because the candidate source hunk was removed and `git diff -- crates/fp-frame/src/lib.rs` is empty relative to the kept `.75` commit.

## Route

Do not continue constructor-bypass micro-levers for this workload. The paired result shows the constructor bypass is below the keep threshold. The next useful route is a deeper mask-certificate primitive: reduce the remaining safe-Rust bool-slice scan cost itself, such as wider block verification or carrying a reusable mask witness from boolean-producing operations, while preserving exact row order, metadata, dtype/null/NaN/f64-bit behavior, and all fallback semantics.

## Artifacts

- `tests/artifacts/perf/uza0476_base_*`
- `tests/artifacts/perf/uza0476_after_*`
- `tests/artifacts/perf/uza0476_pair_*`
- `tests/artifacts/perf/uza0476_test_affine_prevalidated_constructor.txt`
