# Morsel-parallel group-build for dense Int64 groupby (bead `gsr9j`)

- Agent: BlackThrush · Date: 2026-07-31 · Bead: `br-frankenpandas-gsr9j`
- Changed: `crates/fp-frame/src/lib.rs` — new `par_dense_gids_direct`, wired into
  `int64_dense_grouping` (and `dense_gids_from_i64`)
- ELF under test: `14f53c8b723438dd7f0b9ab899a66c37dea8f2663f384233324d6d0d6d79ffc2`
  built from clean `HEAD` + this overlay via `rch exec`, on `vmi1153651` (940 s)
- Baseline ELF: `e231b31837875ba599db08148b2499afcad4c792ddc6283c6038dee832a46d86`

## What was serial, and why it mattered

`int64_dense_grouping` assigned dense group ids in **one serial pass over n rows**
with a loop-carried dependency — `ngroups` increments in first-seen order — so it
pinned a single core while the rest idled. That pass runs **once, before**
`dense_aggregate_emit`, and is shared by every single-key dense reduction reached
from five call sites: `sum`/`mean`/`count`/`min`/`max`/`std`/`var`/`median`/
`prod`/`first`/`last`, plus the `agg_list` and transform entries.

`debc0f8f7` had already parallelised the *per-column* fold and got 2.1-2.5x on
`median`/`std`/`var`, but the cheap one-pass aggs only moved 1.07-1.33x.
`dense_aggregate_emit`'s own comment says why:

> cheap 1-pass aggs (sum/mean/min/max/prod) 1.07-1.33x — **their cost is the
> shared serial group-build, not this fold**

With one value column — which is what the harness's groupby workloads use — the
column-parallel fold buys nothing at all.

## The bit-identity problem, and the fix

Bead `gsr9j` proposed this lever and named the blocker exactly:

> a parallel-then-merge gid assignment (partition rows, local first-seen, merge
> gid tables) — **tricky to keep first-seen order bit-identical**

It is. Naive per-chunk numbering renumbers the groups, which changes `order`, the
output labels and the `sort=True` ordering.

**The fix is to never merge gid tables.** Each worker records, for its own row
range, the **lowest row index** at which it saw each key offset. Those merge by
`min`, and gids are then handed out in **ascending first-row order** — which *is*
the serial first-seen order, by construction. `key_of_gid` is recovered as
`min + offset`, exact because `offset = k - min`.

Consequences, stated precisely:

- `gid_per_row`, `ngroups`, `key_of_gid` are unchanged ⇒ `order`, output labels,
  `sort=True` ascending-key ordering and every downstream aggregate are unchanged.
- **No float arithmetic is reordered anywhere in this change.** The value folds
  are untouched; only the integer group numbering is parallelised. This is why
  the change is bit-identical rather than merely "close".

Two parallel passes replace one serial pass, so the ceiling is **~T/2**.

## Gating

Runs only where it pays and stays cheap; otherwise the original serial builder
runs unchanged, as it also does if any worker panics:

| gate | value | why |
|---|---|---|
| minimum rows | `2^18` | below this, thread setup dominates |
| maximum direct-address range | `2^16` | bounds per-worker scratch |
| worker cap | `2^21 / range` entries | total scratch bounded regardless of key range |
| worker cap | `n / 2^16` | keeps ≥64k rows per worker |

## Correctness evidence

**Randomised equivalence, 400 trials × 6 worker counts (1,2,3,7,16,64):
zero mismatches** on `(gid_per_row, ngroups, key_of_gid)` — covering degenerate
ranges (1, 2), wide ranges, negative minimums, and singleton/empty row counts.

Shipped as two `#[cfg(test)]` regression tests (seeded LCG, no `rand` dependency,
so a failure is reproducible) — **both green**:

```
running 2 tests
test par_dense_gids_bit_identity::group_numbering_follows_global_first_appearance ... ok
test par_dense_gids_bit_identity::parallel_gids_match_serial_first_seen_numbering ... ok
test result: ok. 2 passed; 0 failed; 3213 filtered out
```


- `parallel_gids_match_serial_first_seen_numbering` — straddles the `2^18` gate so
  both the parallel path and the `None` fallback are exercised.
- `group_numbering_follows_global_first_appearance` — a key seen once at row 0 and
  never again must still get gid 0. This is precisely the case naive per-chunk
  numbering gets wrong, so it fails loudly if anyone "simplifies" the merge.

### ⚠️ A witness that does NOT work — recorded so nobody repeats it

I first tried to witness bit-identity with `fp-bench`'s per-run `checksum` field
and got **16/16 IDENTICAL** across eight groupby workloads at 100k and 1M. That
proof is **worthless**, and the tell is that every one returned the same value
(`4957dea0fe3e2ed1`) — `median` cannot legitimately share a digest with `count`.

`crates/fp-bench/src/main.rs:746`:

```rust
*checksum = checksum.rotate_left(9)
          ^ (std::mem::size_of_val(&result) as u64)
          ^ 0x9e37_79b9_7f4a_7c15;
```

It mixes in the **size of the result type**, not the result data. It is an
anti-dead-code-elimination guard and is value-blind: it would agree happily while
a change corrupted every value, since the result type is unchanged. Any parity
claim resting on this field needs re-deriving. Reported to its owner.

## MEASURED — it lost. Reverted.

vs **live pandas 2.2.3, same invocation**, corrected three-clause gate, on
`threadripperje` (64 physical / 128 logical, performance governor, quiet):

| binary | workload @1M | ratio vs pandas | fp p50 |
|---|---|---:|---:|
| HEAD baseline | `groupby_mean_float64` | **3.587x FASTER** | 2456.9 us |
| this lever | `groupby_mean_float64` | 3.217x FASTER | 2765.4 us |
| HEAD baseline | `groupby_sum_int64` | **4.341x FASTER** | 2104.5 us |

**The lever is 12.6% SLOWER than the code it replaced.** Bit-identical, verified,
and still a regression — so it goes.

**Why, and it is worth keeping:** the lever *adds* a second read of the key array
to buy parallelism. At 1M rows the keys plus a 100-entry direct-address table are
cache-resident, so the serial build was already running near memory speed; there
was no stall for extra cores to hide. Parallelism was spread over work that was
not the bottleneck, and the extra pass was pure cost. Bead `gsr9j`'s own closing
line — *"Not obviously worth it"* — was right, for a reason it did not state.

The diagnosis is the useful output: the cost is **memory traffic**, not core
starvation. A one-column `groupby(int64).mean()` walks memory four times and
materializes an n-element `gid_per_row` (8 MB at 1M) purely to hand the gid to
the very next pass. That points at deleting the traffic rather than parallelising
it — see the fused-fold lever, which removes both the write and its read-back.

These are also the **first same-invocation vs-pandas groupby numbers this repo
holds**: FrankenPandas at HEAD is 3.59x pandas on `mean` and 4.34x on `sum` at 1M.

## Expected magnitude — stated before measuring

This is **one of three serial O(n) passes** in the op: the `i64_dense_histogram_range`
min/max scan, this group build, and the value fold. Amdahl therefore caps the
whole-op gain near **~1.5x**, and this commit does not claim multiples.

The f64 fold **cannot** be row-parallelised bit-identically — splitting
`acc[g] += v` across threads reassociates the addition and moves the last ULP. The
bit-identical route to multiples is a stable radix partition of rows by gid so
each thread owns whole groups and sums them in row order; that is the next lever
and is independent of how this one measures.
