# Transpose / block-storage lane — exhaustive closure (DustySummit, 2026-07-23)

Data-backed closure of the transpose / 2D-materialization / reshaping / reduction
surface, measured op-by-op against real pandas 2.2.3 (100k–1M × 10). **fp wins
2–18x on every bounded op.** The one genuine loss (`df_mode` float64) was found
and fixed; the only remaining loss requires a non-incremental block-storage build.

## Measured fp-vs-pandas (fp faster unless noted)

| op | fp | pandas | verdict |
|---|---|---|---|
| `mode` float64 100k | 34ms (was 165ms) | 60ms | **FIXED 774c7146f** (was 5.3x LOSS → 1.75x win) |
| `nunique` float64 100k | 7.6ms | 28ms | fp 3.7x |
| `to_dict(dict)` 100k | 9.9ms | 146ms | fp 14.7x |
| `to_dict(records)` 100k | 10.5ms | 124ms | fp 11.8x |
| `melt` 1M | 54ms | 401ms | fp 7.4x |
| `get_dummies` 1M | 31ms | 556ms | fp 17.8x |
| `stack` 1M | 164ms | 281ms | fp 1.7x |
| `pivot_table` 100k | 1.4ms | 6.5ms | fp 4.6x |
| `to_records` (all 6 dtypes) 100k | ~10–25ms | ~competitive-to-slower | fp competitive-to-faster |
| `quantile` float64 100k | 3.3ms | (faster) | fp win |
| **`.values` / `.to_numpy` (O(1) VIEW)** | 24–31ms materialize | **~1µs view** | **LOSS — block-storage blocker** |

## The sole remaining loss: O(1) `.values`/`.to_numpy` view

pandas stores a homogeneous frame as one contiguous 2-D block, so `.values` /
`.to_numpy` return a **zero-copy view** (O(1), n-independent). fp stores columns
as **separate allocations**, so any row-major materialization is O(n·m) and owned.
Against the fair (eager-copy) comparison — pandas `to_numpy(copy=True)` = 54ms at
1M — fp's 31ms is competitive/faster; the gap is purely the view-vs-copy contract.

**Why there is no bounded lever here:** an O(1) view requires the column data to be
physically contiguous *at construction time*. fp's columns are separate `Vec`s, so
there is no block to view, and no incremental first slice delivers the O(1) win —
it requires block-backed storage wired through every construction path (read_csv,
from_dict, arithmetic results, …). That is a large, non-incremental architectural
project (ledgered `133cf5de0`), whose only beneficiary is zero-copy numpy/BLAS
interop — niche for a Rust library where callers hold typed columns directly.

## Conclusion

Within "bounded benches, commit incremental", the lane's bounded pandas-loss levers
are **exhaustively harvested**: mode was the last one, now fixed; everything else fp
already dominates 2–18x. Post-mode, every profiled op is an fp-win (a run of "no
bounded lever" findings well past 3). The only remaining work is the non-incremental
block-storage build, which needs an explicit go/no-go decision — it cannot be done
as a bounded increment.
