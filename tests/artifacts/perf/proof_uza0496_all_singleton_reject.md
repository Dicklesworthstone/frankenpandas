# br-frankenpandas-uza04.96 all-singleton str groupby direct aggregation reject

Date: 2026-06-13
Owner: LavenderStone
Base commit: 7bc36cba2ab86fa037d1a8ec64d32f4d5ce9106a

## Profile-backed target

`br-frankenpandas-uza04.96` targets the dense string groupby path where
near-all-distinct keys enter `DataFrameGroupBy::aggregate_str_dense`, MSD-sort
all key spans, then build output key bytes and dense aggregate columns.

Prior phase attribution in
`tests/artifacts/perf/blackthrush_str_groupby_dense_phase_profile.md` measured
the degenerate all-distinct path as roughly:

- `group`: ~11 ms
- `out_idx`: ~11 ms
- `agg(count)`: ~3 ms
- `frame`: ~4 us

Dynamic `perf stat` is unavailable in this environment:
`perf_event_paranoid=4`.

## Lever attempted

One rejected source lever was tested and then removed:

- Detect all-singleton sorted runs after the existing MSD permutation.
- For that exact shape, skip `gid_per_row` allocation/fill and skip dense
  accumulator passes.
- Emit each aggregate by the one-row semantics:
  - `count`: `Int64(1)`
  - `var`/`std`: `Null(NaN)`
  - `sum`/`mean`: the same one-row left fold (`0.0 + v`, then `/ 1.0` for mean)
  - `prod`: the same one-row product fold (`1.0 * v`)
  - `min`/`max`/`first`/`last`/`median`: the representative row value/coercion.

The runtime hunk was rejected and is not retained.

## Golden proof

Baseline and candidate hashes matched exactly for the hash rows in
`uza0496_baseline_golden_sha256.txt` and
`uza0496_candidate_golden_sha256.txt`; `cmp_status=0` in
`uza0496_golden_hash_rows_cmp.txt`.

Hashes:

```text
str_groupby_count 1000   77a2baa488f3b39462a15e4cc2223a884afb0d48e088b2ed14b0c2138f8a1894
str_groupby_count 200000 d5cea8c5844bb962c8958f0a94d53a9e7d6b452a55ccaa51c5addb6000c0c46e
str_groupby_sum   1000   a53b6ca20edea8eafabefe76ee5c12bd98d116d02d6372a93707fdc258f1c80d
str_groupby_sum   200000 a28f5534bd1a01e792695b5f80aa877724803caff9bbed0b6fc86807e20b5412
str_groupby_std   1000   33cf06ed6e0f6604c0fb7fb7c91f029e53cf80570c5a987ad4f4382d2e561343
str_groupby_std   200000 72e4b0378ad8409249711f58264e5c4cd64109f5c6f9d684d5e2c419da22a5f5
```

## Timing gate

Candidate build/check ran through RCH on `vmi1227854`; the initial baseline
build command used `rch exec` but failed open locally because no workers were
admissible at dispatch. Timings below compare the retrieved baseline and
candidate release-perf binaries with hyperfine on the same host.

`tests/artifacts/perf/uza0496_pair_all_singleton_candidate.json`:

| Scenario | Baseline | Candidate | Result |
|---|---:|---:|---:|
| `str_groupby_count 200000 6` | 219.1 ms +/- 14.6 | 226.6 ms +/- 40.8 | 0.97x, slower/noisy |
| `str_groupby_sum 200000 6` | 226.2 ms +/- 27.6 | 214.6 ms +/- 19.7 | 1.05x, sub-bar |
| `str_groupby_std 200000 6` | 265.2 ms +/- 26.9 | 267.4 ms +/- 24.6 | 0.99x, slower/noisy |

Baseline shape controls in
`tests/artifacts/perf/uza0496_baseline_str_groupby_degenerate.json`:

- `str_groupby_count 200000 6`: 236.8 ms +/- 43.4
- `str_groupby_sum 200000 6`: 189.1 ms +/- 11.3
- `str_groupby_std 200000 6`: 289.4 ms +/- 13.3
- `str_groupby_count_lowcard 200000 20`: 99.8 ms +/- 10.7
- `str_groupby_sum_lowcard 200000 20`: 100.7 ms +/- 6.1

## Decision

Reject. Behavior was unchanged, but the lever failed the performance gate:
two measured scenarios were neutral/slower and the only positive row was
approximately 1.05x. Score is below 2.0.

Do not retry singleton aggregation, gid allocation removal, or accumulator
micro-specialization for this path. The next pass should attack a different
primitive:

- First fix the benchmark-shape artifact so `build_str_key_frame` honors
  `key_cardinality` for profiling.
- Then re-profile the realistic low-cardinality hash-group branch.
- Candidate deeper primitive: dictionary-coded string groupby using a
  Swiss-table/ART-style key intern table that emits stable group codes and
  contiguous output key storage in one pass, preserving sorted output order by
  sorting only distinct interned keys.
