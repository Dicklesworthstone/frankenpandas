# br-frankenpandas-dwmu9 — typed f64 output for Int64 counting-sort Column::rank

## Lever (one)
`Column::rank`'s all-valid bounded-range Int64 fast path computes ranks in O(n)
via a value histogram + prefix sums, but then materialized the result as
`Vec<Scalar::Float64>` (one heap-boxed Scalar per row) and rebuilt the column
through `Self::new` (which re-validates every Scalar). For 200k rows that
per-row boxing + revalidation dominated — ~10.2 ms/iter, dwarfing the O(n)
counting sort itself. The sibling Float64 radix path already builds typed output
via `from_f64_values`; this brings the Int64 path to parity.

Change: accumulate into a contiguous `Vec<f64>` and return
`from_f64_values(ranks)` instead of `Vec<Scalar>` + `Self::new`.

## Isomorphism proof
- Every row of an all-valid Int64 column is ranked (the loop covers all `i in
  0..len`; `as_i64_slice` ⟹ no nulls), and every rank value
  (`average`/`min`/`max`/`first`/`dense`) is a finite 1-based ordinal or an
  average of two — NEVER NaN/inf.
- `from_f64_values` over a NaN-free vector marks every slot valid and stores the
  exact f64 bits — identical column (dtype Float64, all-valid, same values) to
  `Self::new(DType::Float64, Vec<Scalar::Float64>)`, whose validity comes from
  `Scalar::is_missing` (finite f64 ⟹ valid).
- Rank arithmetic (`start_rank`/`end_rank`, tie midpoints, dense ordinals,
  first-occurrence counter), method/direction handling, and value bits are
  untouched. No unsafe.

## Golden (byte-identical before==after)
`bench_rank_i64` digests f64 rank bits + per-row validity for all 5 methods ×
{asc, desc}, n=200000 (FNV-1a64), all 10 identical before and after:
average `e5f512524f5b5225`/`f219a0a9ff24f325`; min `603a0684489eabe5`/`499f4e3e5f828b25`;
max `1eeb183e6f69db85`/`7742b958e956cd45`; first `d4ec6568dd9f1da5`/`4e69e93fa568a049`;
dense `933fa2ac74265365`/`105f5e992b5e06e5`.

## Benchmark (n=200000, rank("average"), op-only loop)
- min-of-5 internal per-iter: before ~10.2 ms → after ~0.69 ms = **~14.8×**.
- hyperfine -N paired (200 iters): forward **11.65× ± 0.51**, reversed
  **11.73× ± 0.71** faster. Ordering-independent.

Profile-backed: this is the i64 kernel behind `groupby_rank` (the 12 ms/200k
biggest groupby gap in the perf sweep). Score ≫ 2.0.

## Gates
- fp-columnar lib 404/0 (incl. 21 rank tests: average/min/max/first/dense ties,
  null inputs, descending).
- clippy -D warnings clean; fmt clean.
