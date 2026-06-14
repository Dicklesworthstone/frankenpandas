# br-frankenpandas-uza04.127 - df_dot worker cap 32 rejection

## Target

After the `.124` output chunk keep and subsequent tile-width/unroll rejects,
current-main routing still showed `df_dot 100000x6` as the dominant checked
lane:

- `df_dot 100000x6`: 122.626 ms/iter
- next-largest checked lane, `str_sort_chain 100000x10`: 14.839 ms/iter

Local parallelism is 64. This pass tested a row-band scheduling primitive:
cap `DataFrame::dot` worker fanout at 32 instead of 64.

## Isomorphism Proof

The candidate changed only row-band scheduling granularity. Each output row
still belonged to exactly one band, output columns were assembled in the same
row and column order, and every output cell `C[i][j]` still folded `l = 0..k`
in ascending order with the same separate f64 multiply and add. Edge tiles,
NaN/null policy, tie behavior, and RNG state were unchanged.

## Golden Output

Baseline and candidate matched exactly:

```text
df_dot 2000:
ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535

df_dot 5000:
04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d
```

Artifacts:

- `tests/artifacts/perf/lavender_df_dot_worker32_base_golden_2000.txt`
- `tests/artifacts/perf/lavender_df_dot_worker32_base_golden_5000.txt`
- `tests/artifacts/perf/lavender_df_dot_worker32_after_golden_2000.txt`
- `tests/artifacts/perf/lavender_df_dot_worker32_after_golden_5000.txt`
- `tests/artifacts/perf/lavender_df_dot_worker32_after_golden_check.txt`

## Benchmark

Baseline build used the current-main release-perf `perf_profile` binary from a
crate-scoped RCH/fail-open build. Candidate build completed remotely on
`vmi1227854`; artifacts were retrieved and paired timing used local baseline
and candidate binaries from the same host.

Standalone baseline:

```text
df_dot 100000x6: 755.8 ms +/- 25.0 ms
```

Paired forward:

```text
baseline: 729.7 ms +/- 8.9 ms
worker32: 803.2 ms +/- 10.9 ms
```

Paired reversed:

```text
worker32: 812.8 ms +/- 9.8 ms
baseline: 753.4 ms +/- 13.2 ms
```

The existing 64-worker path was 1.08x to 1.10x faster in both pair orders.

## Verdict

Rejected. Score `< 2.0`; the source hunk was removed before closeout.

The user-time reduction confirms less parallel work, but wall time regressed.
For this shape, the 64-way row-band schedule still wins enough latency to
offset its overhead. Do not retry simple lower worker caps. The next route
should target data movement or semantic-witness costs, such as finite-input
NaN-witness elision or chunk-aware consumers, rather than reducing compute
parallelism.
