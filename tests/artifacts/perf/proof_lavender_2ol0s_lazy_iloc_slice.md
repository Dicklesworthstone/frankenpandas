# br-frankenpandas-2ol0s lazy iloc_slice proof

## Change

Route `DataFrame::iloc_slice(start, stop)` for non-row-MultiIndex frames through a contiguous range path instead of materializing `start..stop` positions. The path slices the row index with a lazy materialized-label view and slices each column through the existing contiguous column range primitive.

The perf harness now benchmarks the public `iloc_slice` API for the `iloc_slice` scenario instead of benchmarking a materialized position-list `iloc`.

## Hotspot Evidence

- Bead: `br-frankenpandas-2ol0s`.
- Baseline diagnosis: after the shipped iloc(list) affine fix, `iloc_slice` remained about 11x slower than pandas in the fp-bench matrix. The rejected affine-route attempt showed the residual was common frame/index construction cost, not the column gather algorithm.
- Selected primitive: zero-copy/view-style slice metadata, aligned with graveyard `8.2 Vectorized Execution` columnar view discipline and `0.16 Proof-Carrying Artifact` evidence discipline.

## Benchmarks

Same-worker paired `perf_profile iloc_slice 100000 50`:

- Forward: base `25.7 ms +/- 1.8 ms`, candidate `17.1 ms +/- 1.8 ms`, candidate `1.50x +/- 0.19x` faster.
- Reversed: candidate `16.6 ms +/- 1.8 ms`, base `25.9 ms +/- 2.2 ms`, candidate `1.56x +/- 0.21x` faster.

Bench-runner matrix:

- `indexing/iloc_slice` 100k p50: `0.905 ms -> 0.002 ms`.
- `indexing/iloc_slice` 100k p95: `0.975 ms -> 0.003 ms`.
- `indexing/iloc_slice` 10k p50: `0.520 ms -> 0.002 ms`.
- `indexing/iloc_slice` 10k p95: `0.663 ms -> 0.003 ms`.

Score: Impact 4 * Confidence 5 / Effort 2 = `10.0`; accepted.

## Golden SHA256

Baseline:

```text
a0a28225359ec220a02901f40e2eed1c0b0e53f6cdb24fcffb700d9ebb5e1237  tests/artifacts/perf/lavender_2ol0s_base_golden_iloc_slice_5000.txt
22a1973e9fc0eda8e75a133d70e0b8bf82139bcf3c96559910149e8c91b27463  tests/artifacts/perf/lavender_2ol0s_base_golden_iloc_slice_100000.txt
```

Candidate:

```text
a0a28225359ec220a02901f40e2eed1c0b0e53f6cdb24fcffb700d9ebb5e1237  tests/artifacts/perf/lavender_2ol0s_candidate_golden_iloc_slice_5000.txt
22a1973e9fc0eda8e75a133d70e0b8bf82139bcf3c96559910149e8c91b27463  tests/artifacts/perf/lavender_2ol0s_candidate_golden_iloc_slice_100000.txt
```

All diff artifacts are empty:

```text
0 tests/artifacts/perf/lavender_2ol0s_golden_diff_5000.txt
0 tests/artifacts/perf/lavender_2ol0s_golden_diff_100000.txt
0 tests/artifacts/perf/lavender_2ol0s_golden_iloc_slice_diff.txt
```

## Isomorphism Proof

- Ordering preserved: yes. Slices still enumerate rows in ascending positional order from resolved `start_pos..end_pos`.
- Tie-breaking unchanged: yes. There is no comparison or tie decision; original row order is preserved.
- Index semantics unchanged: yes. `Index::slice` preserves the index name via `propagate_name`, and lazy materialized-label slices expose the same `as_slice` ordering for equality, serialization, and iteration.
- Row MultiIndex behavior unchanged: yes. The new fast path is limited to `row_multiindex.is_none()`. Row MultiIndex frames continue to use the existing position-list path.
- Bounds semantics unchanged: yes. `iloc_slice` resolves negative, open, and out-of-range bounds before calling the contiguous path. Empty slices still return empty columns and an empty index.
- Floating-point unchanged: yes. No numeric values are transformed; column slicing either reuses typed backing or materializes the same contiguous scalar range.
- RNG unchanged: N/A. No RNG is used.

## Validation

- `git diff --check -- crates/fp-index/src/lib.rs crates/fp-frame/src/lib.rs crates/fp-conformance/examples/perf_profile.rs`: pass.
- `rch exec -- cargo test -p fp-index slice_of_materialized_index_keeps_shared_label_view --lib`: pass; RCH fell back local because no workers were admissible.
- `rch exec -- cargo test -p fp-frame dataframe_iloc_slice --lib`: pass; RCH fell back local because no workers were admissible.
- `rch exec -- cargo check -p fp-index -p fp-frame --lib`: pass on `vmi1153651`; custom target artifact retrieval emitted an rsync warning after command success.
- `rch exec -- cargo clippy -p fp-index -p fp-frame --lib -- -D warnings`: pass on `vmi1153651`.
- `rch exec -- cargo check -p fp-conformance --example perf_profile`: pass on `vmi1153651`.

## Risk And Rollback

Primary risk: index lazy views must not outlive backing storage. Countermeasure: the view owns an `Arc<Vec<IndexLabel>>`, so backing storage stays alive as long as any slice exists.

Rollback: `git revert <commit>` restores eager index slicing and the previous perf harness route.
