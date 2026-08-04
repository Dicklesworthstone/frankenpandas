# Fusing the dense Int64 group-build into the value fold — measured WIN

- Agent: BlackThrush · Date: 2026-07-31
- Changed: `crates/fp-frame/src/lib.rs` — new `try_fused_int64_dense`, tried first
  in `aggregate_int64_dense`
- Host: `threadripperje` — 64 physical / 128 logical, 512 GB, performance
  governor, quiet
- Baseline ELF `e231b318…` (clean HEAD) · fused ELF `2cfcd3a2…`

## The win

FP-side A/B between the two ELFs, decided by **fp-bench's own interleaved A/A
null control** (paired median ratio must clear 2× the null half-width, and both
A/A null medians must sit within 2% of unity):

| workload | size | baseline p50 | fused p50 | ratio | verdict |
|---|---|---:|---:|---:|---|
| `groupby_mean_float64` | 1M | 2154.2 µs | 1422.9 µs | **1.514×** | FASTER |
| `groupby_sum_int64` | 1M | 2218.3 µs | 1367.0 µs | **1.623×** | FASTER |
| `groupby_count` *(control)* | 1M | 2364.6 µs | 2473.7 µs | 0.956 | NULL_UNDECIDABLE |
| `groupby_mean_float64` | 10M | 24914.7 µs | 15064.6 µs | **1.654×** | FASTER |
| `groupby_sum_int64` | 10M | 24237.9 µs | 14080.3 µs | **1.721×** | FASTER |
| `groupby_count` *(control)* | 10M | 24064.4 µs | 23094.8 µs | 1.042 | NULL_UNDECIDABLE |

`groupby_count` is a **built-in control**: `count` is not in the fused
`{sum, mean}` set, so the fused binary must leave it alone — and it does, landing
undecidable at both sizes. A change that had accidentally sped up *everything*
would have been measuring the host, not the lever.

## vs LIVE pandas, same invocation — the gated before/after

Same host, corrected three-clause gate, **all three clauses satisfied on every
row** (`clauses=111`), pandas 2.2.3 in-process, shipped ELF `4225b3fb…`:

| binary | workload @1M | ratio | fp p50 | pandas p50 |
|---|---|---:|---:|---:|
| HEAD baseline | `groupby_mean_float64` | 3.513x | 2454.25 µs | 8621.96 µs |
| **shipped (fused)** | `groupby_mean_float64` | **6.968x** | **1361.73 µs** | 9488.30 µs |
| HEAD baseline | `groupby_sum_int64` | 3.953x | 2089.86 µs | 8261.93 µs |
| **shipped (fused)** | `groupby_sum_int64` | **6.124x** | **1367.76 µs** | 8375.54 µs |

**Both ops go from ~3.5-4x pandas to ~6x pandas.**

⚠️ **Read the `mean` ratio jump carefully.** Its pandas arm drifted between the
two invocations — 8621.96 → 9488.30 µs, about +10% — so part of 3.513 → 6.968 is
incumbent variance rather than our gain. Holding the incumbent fixed at the
baseline invocation's value gives **~6.3x**, and that is the number to quote.

**The `sum` pair is the clean one and it validates the claim.** There the pandas
arm moved only +1.4% (8261.93 → 8375.54), and the two independent views agree:

| view | value |
|---|---|
| FP-side gain | 2089.86 / 1367.76 = **1.528x** |
| ratio gain | 6.124 / 3.953 = **1.549x** |

Those agree to within 1.4%, which is exactly the incumbent drift — an internal
consistency check that the effect is the lever and not the host. Drift-corrected,
the fused `sum` ratio is 8261.93 / 1367.76 = **6.04x**.

## What it does

A one-column `df.groupby(int64_key).mean()` walked memory four times:

1. `i64_dense_histogram_range` — read the keys
2. `int64_dense_grouping` — read the keys again, **write an n-element `Vec<usize>`**
3. `dense_aggregate_emit` — read the values, **read that vector back**

At 1M rows `gid_per_row` alone is **8 MB written and 8 MB read**, and it exists
only to hand the gid to the very next pass. The fused path accumulates straight
into direct-address **slot** bins while scanning, so `gid_per_row` is never
materialized, and recovers first-seen gid order at the end from a `first_row[]`
witness.

## Why it is bit-identical

The accumulation still runs in ascending **row** order into a per-group bin:
`acc[gid_per_row[row]] += v` becomes `acc[slot(key[row])] += v`, and slot ↔ gid is
a bijection. Each group's values are therefore added in exactly the same sequence
with exactly the same intermediate rounding. **Nothing is reassociated** — which
is precisely what makes this legal where row-range parallelism of an f64 sum is
not.

`sort=True` orders by ascending key, which for a direct-address table is
ascending slot; `sort=False` takes slots in ascending `first_row`, which is the
first-seen numbering.

Scope is deliberately narrow: exactly one value column, func in `{sum, mean}`, and
an all-valid Float64 value column. `as_f64_slice` already gates on
`validity.all()`, so nullable columns cannot reach it and the skipna branches are
untouched. `median`/`std`/`var` need a second pass and stay on the split path.

**Correctness:** `cargo test -p fp-frame --lib groupby` — **211 passed, 0 failed**.

## How this lever was found: by a failure

The first attempt parallelised the group build itself (bead `gsr9j`) and
**measured 12.6% SLOWER** than the code it replaced — 3.217× vs 3.587× against
live pandas at 1M — despite being bit-identical and fully verified. It has been
reverted.

The reason is the whole lesson: that lever **added** a second read of the key
array to buy parallelism. At 1M the keys plus a 100-entry direct-address table
are cache-resident, so the serial build was already running near memory speed and
there was no stall for extra cores to hide. **The cost was memory traffic, not
core starvation** — so the payoff came from deleting the traffic, not spreading
it. Same op, opposite strategy, 1.5-1.7× instead of 0.87×.

## Follow-up that was REJECTED: extending the fusion to `count` and Int64 columns

The obvious next step — widen the fused gate to `count` and to Int64 value columns
— was implemented, passed **211/211** groupby tests, and **measured worse**. It is
not shipped (stashed as `rejected-count-i64-extension-measured-slower`).

FP-side A/B, shipped ELF `4225b3fb…` vs extended ELF `b40f3117…`:

| workload | size | shipped | extended | ratio |
|---|---|---:|---:|---|
| `groupby_count` | 1M | 2520.6 µs | 2516.3 µs | 1.002 — no change |
| `groupby_mean_float64` | 1M | 1430.0 µs | 1793.4 µs | **0.797** |
| `groupby_sum_int64` | 1M | 1458.9 µs | 1799.9 µs | **0.811** |
| `groupby_count` | 10M | 24674.8 µs | 24034.7 µs | 1.027 — no change |
| `groupby_mean_float64` | 10M | 15865.7 µs | 17717.1 µs | 0.895 |
| `groupby_sum_int64` | 10M | 14452.4 µs | 16666.8 µs | **0.867 SLOWER** |

Two separate findings, and both are worth keeping:

1. **`count` did not move at all**, at either size. It never reaches the fused
   path — `try_count_dense` intercepts `count` earlier in `aggregate_named_func`.
   Widening the fused gate to include `"count"` was dead code. **Check what
   already intercepts a func before widening a gate to cover it.**
2. **The already-winning `sum`/`mean` paths got ~20% SLOWER.** Adding the Int64
   arm turned a single tight typed loop into a `match` over two `Option` slices
   with an extra range-sized `i128` accumulator allocated on every call — cost
   paid by the common f64 path to serve a case that was never measured to need
   it. The lesson mirrors the parallel-gid failure: **a hot fused loop is fragile;
   generalising it has a price, and that price has to be measured, not assumed.**

⚠️ **Do not witness this with `fp-bench`'s `checksum` field.** It hashes
`size_of_val(&result)`, not the data, and returns the same value for every
groupby workload — it is an anti-DCE guard, not a digest. The correctness
evidence here is the 211-test run, not that field.
