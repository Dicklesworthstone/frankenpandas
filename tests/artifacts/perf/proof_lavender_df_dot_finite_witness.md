# br-frankenpandas-uza04.128 - df_dot finite-bound NaN witness rejection

## Target

After `.124` and the subsequent BI/BJ/unroll/worker-cap rejects, current-main
routing still showed `df_dot 100000x6` as the dominant checked lane:

- `df_dot 100000x6`: 118.058 ms/iter
- next-largest checked lane, `str_sort_chain 100000x10`: 13.059 ms/iter

This pass tested a semantic-witness primitive: prove before GEMM that all
inputs are finite and `k * max_abs_a * max_abs_b` cannot overflow f64. If that
proof holds, output cells cannot become NaN, so per-output `value.is_nan()`
witness updates can be skipped and all-valid chunks can be built directly. If
the proof fails, the existing exact witness path is used.

## Isomorphism Proof

The candidate did not change arithmetic or output construction order. Every
output cell still folded `l = 0..k` in ascending order with the same separate
f64 multiply and add. The fast path was guarded by a conservative proof:

- every A and B operand must be finite;
- if either max absolute value is zero, all products are zero;
- otherwise `max_abs_a <= f64::MAX / max_abs_b` and
  `max_abs_a * max_abs_b <= f64::MAX / k`.

Those conditions ensure no product or k-term sum can overflow to `+/-inf`, so
finite inputs cannot create `NaN`. Any non-finite or potentially overflowing
input falls back to the existing per-output witness path.

## Golden Output

Baseline and candidate matched exactly:

```text
df_dot 2000:
ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535

df_dot 5000:
04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d
```

## Benchmark

Baseline build used the current-main release-perf `perf_profile` binary from a
crate-scoped RCH/fail-open build. Candidate build also failed open locally.

Standalone baseline:

```text
df_dot 100000x6: 736.4 ms +/- 11.7 ms
```

Paired forward:

```text
baseline: 759.0 ms +/- 17.6 ms
finite witness: 862.2 ms +/- 18.2 ms
```

Paired reversed:

```text
finite witness: 877.1 ms +/- 16.8 ms
baseline: 773.6 ms +/- 26.9 ms
```

The existing output-witness path was 1.13x to 1.14x faster in both pair orders.

## Verdict

Rejected. Score `< 2.0`; the source hunk was removed before closeout.

The bound proof is correct but too expensive for this shape: it scans the full
A matrix once to avoid an output-side witness that is cheaper in the current
cache/threading regime. Do not retry pre-GEMM full-input scans for this lane.
The next route should avoid extra full-matrix passes and instead look for a
witness carried by existing typed column metadata or a consumer that can use
chunked output without materializing/scanning.
