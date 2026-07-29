# Hypothesis ledger — trj `df_abs` row partition

| Hypothesis | Status before exclusive run | Evidence / required test |
|---|---|---|
| A literal ten-worker constant caps FrankenPandas | rejects | `par_map_columns_min` uses `available_parallelism().min(ncols)` and the fixture has ten columns |
| The 1M decline after cap 16 is caused by additional FP workers | rejects | Caps 16/32/64/128 all observed exactly ten operation workers |
| The 1M decline is compact-cache versus wide-placement overhead | pending | Hold affinity cardinality and actual workers at ten; compare exact compact and eight-L3 spread masks across 1M/2M/4M/6M/8M/10M |
| The 10M gain from cap 16 to 32 is reduced per-L3 streaming contention | pending | Compare cache/LLC counters and wall time with the same ten workers under compact and spread masks |
| A row-by-column decomposition can remove at least 29.3% at 10M | pending | Named-frame profile must expose enough removable time; then run implementation A/B and live-pandas gate |
| The path is already at the DRAM bandwidth ceiling | pending | Record cycles, task clock, cache/LLC misses, and achieved bytes per second before mutation |
| More than 64 hardware threads helps this streaming kernel | pending | Keep the 64-physical-core and 128-SMT rows separate; accept only median-CI evidence |

## Retry predicates already consumed

The 2026-06-20 ledger row now permits reopening only for a named-frame
large-N bandwidth profile plus a row-chunk design capable of exceeding the
column count. The 2026-07-29 sweep independently names the same row-chunk
predicate and requires at least 29.3% removal for the 5x incumbent target.
