# DEFINE — trj `df_abs` row-partition scaling

## Scenario

Run `DataFrame::abs()` over the canonical all-valid Float64 frame with ten
columns at 1M and 10M rows. The current implementation distributes independent
columns across `available_parallelism().min(ncols)` scoped workers, so the
existing trj artifact observes ten operation workers at every requested cap
from 16 through 128. Profile the exact current 10M ELF before any candidate
timing or landing. If the profile supports it, compare that baseline with one
safe-Rust row-by-column work decomposition that can use more workers without
changing cell values, column order, dtype, index, validity, or finiteness
semantics.

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

## Final decision

**REJECT** the portable scoped-thread row×column candidate and retain the
current column scheduler.

The exclusive production sweep made the requested/actual worker distinction
real: at 10M, the current path stayed at ten actual workers for requested caps
16/32/64/128, while the candidate used 16/32/64/128. More workers did not
remove time:

| requested cap | current workers / p50 | candidate workers / p50 | candidate versus current |
|---:|---:|---:|---:|
| 16 | 10 / 26.465 ms | 16 / 37.778 ms | 42.7% slower |
| 32 | 10 / 20.022 ms | 32 / 29.068 ms | 45.2% slower |
| 64 | 10 / 20.336 ms | 64 / 24.173 ms | 18.9% slower |
| 128 | 10 / 20.680 ms | 128 / 24.527 ms | 18.6% slower |

The best candidate row remained 75.4% above the fixed 13.780 ms campaign
budget and produced only a 2.812x same-invocation pandas ratio. All rows used
50 timed calls per engine, the same fixtures, and stable checksums, so this is
not an iteration-count-shaped loss. The candidate adds scoped-thread and
per-worker allocation work while the number of elementwise `abs` operations
stays fixed.

The full adaptive production sweep was already sufficient to reject the
implementation. The cap-13 threshold series was therefore stopped rather than
spending another exclusive-host window tuning a candidate that lost at every
raised cap. The rejected source was removed; the profile and all raw attempts
remain as the result.

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

## Existing-ELF curve hypothesis

The 1M decline after cap 16 is **not** evidence of additional worker overhead:
the operation probe observed exactly ten FrankenPandas workers at caps 16, 32,
64, and 128. Only the set of CPUs on which Linux may place those ten workers
widens. The historical medians imply the following input-plus-output streaming
lower bounds (ten Float64 input columns plus ten Float64 output columns):

| rows / affinity cap | FP median | minimum bytes moved | effective lower-bound throughput |
|---|---:|---:|---:|
| 1M / 16 | 2.232 ms | 160 MB | 71.7 GB/s |
| 1M / 128 | 3.076 ms | 160 MB | 52.0 GB/s |
| 10M / 16 | 30.095 ms | 1.6 GB | 53.2 GB/s |
| 10M / 128 | 19.482 ms | 1.6 GB | 82.1 GB/s |

Those historical rows predate the mandatory host-wide arm brackets, so the
timings are diagnostic rather than proof of the cause. They predict that
compact placement wins while the 160 MB operation is small enough for cache
locality to dominate, whereas broad placement wins once the 1.6 GB operation
needs aggregate memory bandwidth. The controlled, host-exclusive ten-CPU
compact-versus-spread experiment above must distinguish that prediction from
co-tenant contamination before a candidate row-partition threshold is
attributed to worker-granularity overhead.

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
