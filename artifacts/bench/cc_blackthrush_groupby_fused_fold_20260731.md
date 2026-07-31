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

⚠️ **Do not witness this with `fp-bench`'s `checksum` field.** It hashes
`size_of_val(&result)`, not the data, and returns the same value for every
groupby workload — it is an anti-DCE guard, not a digest. The correctness
evidence here is the 211-test run, not that field.
