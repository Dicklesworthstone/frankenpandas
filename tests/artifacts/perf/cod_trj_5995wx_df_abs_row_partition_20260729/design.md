# Candidate design — portable two-dimensional `df_abs`

## Entry guard

The candidate is a narrow `DataFrame::abs()` fast path. It may run only when:

- every frame column is an all-valid contiguous `Float64` column;
- the row count clears the measured crossover threshold;
- `available_parallelism()` exceeds the column count.

Every other dtype/validity/shape retains the current
`apply_per_column_min(131_072, Column::abs)` path. This keeps mixed, nullable,
Bool, Int64, Datetime-like pass-through, index, and error behavior outside the
new scheduling path.

## Work decomposition

Collect the ten immutable input slices in column order and allocate one
contiguous output `Vec<f64>` per column. Divide the available worker budget
across columns, then divide each column into disjoint contiguous row chunks.
One `std::thread::scope` owns every chunk task, so the total spawned worker
count is the requested budget rather than ten nested pools. Each task performs
only:

```text
output[row] = input[row].abs()
```

No task shares a mutable output range. Reassembly follows the existing column
order. The implementation is safe Rust with no x86 intrinsics; it therefore
keeps the scalar/codegen fallback usable on Apple Silicon.

## Column boundary

Keep the finiteness proof inside `fp-columnar`. A hidden helper accepts source
`Column` references, rejects anything other than all-valid Float64, performs
the row-chunk map, and constructs each result through the existing
finiteness-witness-preserving owned constructor. This avoids exposing a generic
"trust me" constructor to `fp-frame`, avoids a second NaN/finiteness scan, and
retains the current contiguous owned output representation.

Do not use `from_f64_all_valid_chunks`: although all values would have been
computed, changing the output representation would make a drop-only benchmark
understate downstream materialization cost. Do not use nested per-column
thread scopes: at cap 128 that would create ten pools and obscure the actual
worker budget.

## Required proof before timing

- For `-0.0`, finite signs, infinities, and tail chunks, every result value has
  the same `to_bits()` as the current `Column::abs`.
- Dtype, length, validity, finiteness witness, index, and column order match.
- Nullable and mixed frames demonstrably take the existing fallback.
- The untimed operation probe observes more than ten workers at caps above ten.
- The named-frame baseline profile exposes enough removable self-time to make
  the 13.780 ms 10M incumbent budget feasible.

## Threshold rule

Do not choose the threshold from the old 1M/10M endpoints. Measure
1M/2M/4M/6M/8M/10M with the candidate and current path in alternating pairs,
first under controlled ten-CPU compact/spread masks and then at caps
16/32/64/128. Select the smallest row count whose candidate improvement clears
twice the same-invocation A/A log half-width in consecutive measured sizes.
If no monotone crossover exists, reject the automatic gate rather than
hard-coding a convenient endpoint.
