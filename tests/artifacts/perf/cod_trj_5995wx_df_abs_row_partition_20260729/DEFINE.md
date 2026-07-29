# DEFINE — trj `df_abs` row-partition scaling

## Scenario

Run `DataFrame::abs()` over the canonical all-valid Float64 frame with ten
columns at 1M and 10M rows. The current implementation distributes independent
columns across `available_parallelism().min(ncols)` scoped workers, so the
existing trj artifact observes ten operation workers at every requested cap
from 16 through 128. Profile the exact 10M path before source mutation. If the
profile supports it, compare that baseline with one safe-Rust row-by-column
work decomposition that can use more workers without changing cell values,
column order, dtype, index, validity, or finiteness semantics.

## Metric and decision

Primary competitive metric: pandas 2.2.3 median wall time divided by
FrankenPandas median wall time, with both engines run side-by-side in the same
invocation. Directional decisions use the bootstrap median 95% CI with a
two-times A/A null margin. CV is recorded as provenance and has no vote.

The implementation A/B is maintenance evidence only. A campaign win requires
the live pandas incumbent arm in the same invocation.

## Budget

At 10M rows, beat live pandas by at least 5x. The current best trj medians are
19.482 ms for FrankenPandas and 68.902 ms for pandas (3.537x), so the same
pandas time implies a FrankenPandas budget of at most 13.780 ms: at least
29.3% total-time removal from the current best.

## Golden output

For every candidate run, compare every output column with the current
`Column::abs` path, including `to_bits()` for all-valid Float64 values, and
verify identical index, column order, dtype, validity, and finiteness witness
behavior. Retain the harness checksum and process-self executable identity.

## Apples-to-apples matrix

| Axis | Status | Required value |
|---|---|---|
| Workload, rows, columns, dtype, seed | MATCH | Same ten-column generated Float64 frame |
| API | MATCH | `DataFrame::abs()` versus `pandas.DataFrame.abs()` |
| Warmup and paired rounds | MATCH | Three warmups; 25 alternating A/A pairs per engine |
| Host and kernel | MATCH | `threadripperje`; record kernel in every artifact |
| Build profile | MATCH | FrankenPandas `release-perf`; pinned pandas 2.2.3 |
| Affinity | STATE | Exact mask recorded per row |
| Requested / observed workers | STATE | Full 1/2/4/8/16/32/64/128 sweep; report both |
| Scheduler implementation | STATE | Existing column-only versus candidate row-by-column |
| Measurement order | RANDOMIZE | Alternating same-invocation pairs |

This result can support an x86-64-v3 claim on the recorded 5995WX host. It
cannot support an Apple Silicon claim; the implementation must remain portable
so an M4/M5 measurement can be run separately.

## Crossover experiment

Hold the implementation and actual worker count fixed while varying row count
and affinity breadth. This distinguishes work-granularity overhead from
placement effects. In particular, caps 16/32/64/128 in the current ELF all use
ten operation workers, so their 1M divergence is an affinity/topology result,
not evidence that more workers ran. Bracket the crossover at
1M/2M/4M/6M/8M/10M rows. First hold both the implementation and
affinity-cardinality at ten logical CPUs while comparing a compact
physical-core mask with a mask spread across the eight L3 domains (derive and
record the exact CPU IDs from `lscpu -e` after claiming trj). Then run the
requested-cap sweep. This separates placement from worker count; keep the
128-logical-CPU/SMT row separate from the 64-physical-core row.

## Scope boundary

This round does not alter GroupBy, sort, join, pandas, global runtime policy, or
ISA dispatch. GroupBy row partitioning is a separate follow-up vein. No x86
intrinsics or architecture-specific hot-path assumptions are allowed.

## Variance envelope

- Keep every raw sample and A/A ratio.
- Repeat or invalidate a row only when its same-invocation A/A evidence fails;
  never rerun merely to lower CV.
- Do not mix measurements with another trj workload.

## Requester

Owner-directed Lane M campaign work, tracked by `br-frankenpandas-92ttk`.
