# br-frankenpandas-uza04.125 - df_dot BI=8 row micro-tile rejection

## Target

Before rebasing onto the `.124` worker-private chunk keep, `df_dot 100000 6`
remained the dominant measured hotspot after the post-.123 routing matrix:

- `df_dot 100000 6`: 162.746 ms/iter
- next-largest checked lane, `str_sort_chain 100000 10`: 12.395 ms/iter

This pass tested a different GEMM primitive from the rejected output-family
levers on that pre-.124 baseline: widen the private row-band microkernel from
`BI=4, BJ=4` to `BI=8, BJ=4`.

## Isomorphism Proof

The candidate only changed the number of independent rows carried in the
private accumulator tile. For every output cell `C[i][j]`, the inner product
still folded `l = 0..k` in ascending order with the same `acc += a * b`
operation. It did not change column order, row order, tie-breaking, RNG state,
null/NaN policy, dtype construction, or output labels.

Before accepting or rejecting the lever, candidate and baseline golden outputs
were compared byte-for-byte for two sizes.

## Golden Output

Baseline and candidate matched exactly:

```text
df_dot 2000:
ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535

df_dot 5000:
04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d
```

Artifacts:

- `tests/artifacts/perf/lavender_df_dot_bi8_base_golden_2000.txt`
- `tests/artifacts/perf/lavender_df_dot_bi8_base_golden_5000.txt`
- `tests/artifacts/perf/lavender_df_dot_bi8_after_golden_2000.txt`
- `tests/artifacts/perf/lavender_df_dot_bi8_after_golden_5000.txt`
- `tests/artifacts/perf/lavender_df_dot_bi8_after_golden_check.txt`

## Benchmark

RCH candidate build completed on `vmi1227854`; artifacts were retrieved and the
baseline/candidate benchmark binaries were run from the same host. These
measurements compare BI=4 against BI=8 before the `.124` worker-private chunk
keep changed the output materialization path.

Standalone baseline:

```text
df_dot 100000 6: 961.4 ms +/- 10.9 ms
```

Paired forward:

```text
baseline:  954.0 ms +/- 14.1 ms
BI=8:        1.046 s +/- 0.014 s
```

Paired reversed:

```text
BI=8:        1.025 s +/- 0.022 s
baseline:  958.3 ms +/- 85.4 ms
```

The baseline BI=4 kernel was 1.07x to 1.10x faster in both pair orders.

## Verdict

Rejected. Score `< 2.0`; the source hunk was removed before closeout.

The likely cause is register pressure and row working-set width: widening from
four to eight rows increases live accumulator state without reducing the
dominant memory traffic for this `k=6` workload. Do not retry simple BI
widening. The next `df_dot` primitive should be algorithmically different:
for example a transposed/right-packed microkernel, adaptive skinny-k unroll
specialization, or zero-copy output-column materialization that preserves the
same per-cell fold order.
