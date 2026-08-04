# Hypothesis ledger — trj `df_abs` row partition

| Hypothesis | Status before exclusive run | Evidence / required test |
|---|---|---|
| A literal ten-worker constant caps FrankenPandas | rejects | `par_map_columns_min` uses `available_parallelism().min(ncols)` and the fixture has ten columns |
| The 1M decline after cap 16 is caused by additional FP workers | rejects | Caps 16/32/64/128 all observed exactly ten operation workers |
| The 1M decline is compact-cache versus wide-placement overhead | supports as routing evidence | With the implementation and actual workers fixed at ten, compact/spread medians were 2.909/2.940 ms at 1M, effectively tied; this is not a more-worker effect |
| The 10M gain from cap 16 to 32 is reduced per-L3 streaming contention | supports as routing evidence | With ten actual workers, spread measured 22.433 ms versus compact 26.701 ms at 10M; the descriptive crossover appears between 6M and 8M, but the arms were separate invocations and no directional keep is claimed |
| A row-by-column decomposition can remove at least 29.3% at 10M | rejects | The candidate exposed 16/32/64/128 actual workers, yet its best raised-cap median was 24.173 ms at cap 64 versus current 20.336 ms at the same cap and 20.022 ms at cap 32 |
| The path is already at the DRAM bandwidth ceiling | unresolved and no longer admission-critical | The current profile named `Column::abs`, but the rejected candidate did not justify another exclusive counter run; no DRAM-ceiling claim is made |
| More than 64 hardware threads helps this streaming kernel | rejects as a keep | Candidate cap 128 measured 24.527 ms versus 24.173 ms at cap 64; there is no same-invocation evidence that SMT removed time |

## Retry predicates already consumed

The 2026-06-20 ledger row now permits reopening only for a named-frame
large-N bandwidth profile plus a row-chunk design capable of exceeding the
column count. The 2026-07-29 sweep independently names the same row-chunk
predicate and requires at least 29.3% removal for the 5x incumbent target.

## Amount-of-work audit

Every canonical row executed 50 timed calls for FrankenPandas and 50 for
pandas: 25 alternating A/A pairs per engine. Current and candidate used the
same 1M/10M-by-ten Float64 fixtures and stable checksums. The loss is therefore
not an iteration-count or fixture-cardinality mismatch.

The candidate kept the same number of elementwise `abs` applications but
increased orchestration work: one scoped thread and one independently allocated
`Arc<[f64]>` chunk per worker, up to 128, versus at most ten column tasks and
ten owned output buffers in the current path. The measurements establish the
loss; attributing it specifically between thread creation and allocation
requires a candidate profile and remains an inference.

## Final decision

**REJECT** the scoped-thread, worker-private-Arc row×column design. The
production sweep already rejected every raised-cap row, so the planned
fixed-cardinality cap-13 crossover could not rescue this implementation and
was not run. The candidate source was removed after the raw corpus was copied
and hash-verified.
