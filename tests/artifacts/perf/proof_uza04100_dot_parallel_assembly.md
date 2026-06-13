# df.dot output assembly — parallel zero-free column build — KEPT (1.72x)

BlackThrush, 2026-06-13. perf_event_paranoid=4; attribution via env-gated phase
timers in `DataFrame::dot` (reverted before commit).

## Target (phase timers, df_dot 100000: m=100000, k=256, n=256)

After uza04.98 eliminated the A transpose, phase timers revealed the residual
231ms was NOT compute:

```
extract ~0.4ms   compute ~55ms   assemble ~95ms   result_cols ~110ms
```

- `assemble` ~95ms: the code allocated and ZERO-FILLED an n×m `cols` buffer
  (~205MB) then scatter-copied the row-bands into it — a serial double-touch of
  the whole output.
- `result_cols` ~110ms: `Column::from_f64_values` ×256 each runs an O(len) NaN
  scan, all SERIAL.

## Lever (bit-identical)

1. Drop the zeroed `cols` buffer: build each output column directly via
   `Vec::with_capacity(m)` + `extend_from_slice` from the bands in ascending
   row-band order (no zero-fill, single write pass).
2. Fan the per-column build — INCLUDING the `Column::from_f64_values` NaN scan —
   across workers (columns are independent), so the ~110ms serial NaN scan +
   the assembly run in parallel.

Byte-identical: column j is the same C[*][j] values in the same row order;
`from_f64_values` is unchanged.

## Result

- Golden sha256 BYTE-IDENTICAL (df_dot n=2000, n=5000).
- `cargo test -p fp-frame dot`: 12 passed, 0 failed.
- **df_dot 100000: 231.576 → 134.998 ms = 1.72x** (min-of-8, rch worker).

## REJECTED sub-lever (unfiled experiment): per-band contiguous A-pack

I also tried packing each worker band's A into a contiguous `a_panel[l*bw+local]`
to make the microkernel read a contiguous 4-lane slice (avoid the `Vec<Cow>`
deref). Bit-identical, but it REGRESSED the combined result: WITH the pack
df_dot was 1.30x, WITHOUT it 1.72x. The pack adds ~205MB of `a_panel`
allocation per call (64 bands × 3.2MB) whose alloc/free churn outweighs any
kernel gain — the column-major `a_cols[l][row]` kernel was already adequate.
Reverted.

## Next swing (reporting rule)

df_dot is now bounded by OUTPUT-ALLOCATION CHURN, not compute: each call
allocates+frees the 205MB column-major result plus the 205MB row-band
intermediate (and the bench drops the 205MB output every iter). The ~2x bench
ceiling is the cost of materializing+freeing the result. The next lever is to
cut allocation churn — eliminate the row-band intermediate (workers write
directly into a row-major output via `chunks_mut`, then a parallel
column-extract) and/or an output-buffer arena the caller can reuse. Target: fold
the ~205MB intermediate to clear 2x.
