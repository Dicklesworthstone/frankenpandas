# Fused parallel validity/finiteness witness for element-wise float maps

**Agent:** SwiftHill (claude-code / opus-5)
**Date:** 2026-07-31
**Host:** `thinkstation1`, AMD Ryzen Threadripper PRO 5975WX, 32 physical / 64
logical cores, kernel `6.17.0-35-generic`
**Base commit:** `f37890e6a96e3de5d092d86cbe3c35f00afa52ff`

Decision: **KEEP the FP-side self-speedup. The vs-pandas ratio is NOT measured
and no competitive claim is made here.**

> **Ledger-bound entry.** `docs/NEGATIVE_EVIDENCE.md` carried 67 lines of a peer's
> uncommitted work (CyanLynx's math-unary ISA retest) at commit time, and this is a
> shared checkout, so this finding is banked in an artifact rather than staged into
> a file whose pending hunks belong to someone else. Fold it into the ledger once
> that work lands.

## Result class, stated first

This is a `maintenance-self-speedup` (FP-vs-FP), measured under peer load with an
interleaved A/A null control. It is **not** gate-admitted and **not** a live-pandas
comparison. The host-wide exclusivity gate never cleared during this session; see
"Blocked measurement" below. Per the project rule, the workload and the mechanism
are banked and the competitive number is left unclaimed rather than produced by
weakening the instrument.

## The defect

`docs/NEGATIVE_EVIDENCE.md` (2026-07-31, CyanLynx) records the two surviving
math-unary losses against live pandas 2.2.3 at 1M Float64: `sqrt` **0.361x** and
`log` **0.630x**, with `sqrt` observing **eight** FrankenPandas operation threads
against pandas' one. An operation that loses 2.8x while using 8 threads against a
single-threaded incumbent is not ISA-bound; it is doing extra work.

It was. In `crates/fp-columnar/src/lib.rs`,
`typed_float_unary_nullable_owned_par` — the kernel behind `Column::sqrt`,
`Column::exp` and `Column::ln` — ran the expensive map in parallel and then
performed a **second, fully serial pass over the entire output** to derive the
validity/finiteness witness:

```rust
let out = par_map_vec_f64(len, |i| f(data[i]));   // parallel, 8 workers
let mut validity_words = vec![0_u64; len.div_ceil(64)];
for (idx, &y) in out.iter().enumerate() {          // SERIAL, whole output
    all_finite &= y.is_finite();
    if y.is_nan() { all_valid = false; }
    else { validity_words[idx / 64] |= 1_u64 << (idx % 64); }
}
```

At 1M Float64 that serial pass re-reads 8 MB on a dependence the parallel map had
just spent eight threads to avoid. Worse, on the benchmark's strictly-positive
input `all_valid` stays true, so the 128 KB of validity words the pass builds are
**discarded outright** for `ValidityMask::all_valid(len)`. The work was not merely
serial, it was mostly dead.

## The change

One new kernel, `par_map_vec_f64_with_witness`, in which each worker writes its
own value chunk **and** that chunk's packed validity words in the same pass and
returns `(all_valid, all_finite)` as a reduction. `typed_float_unary_nullable_owned_par`
consumes it; the serial pass is deleted.

Chunking is rounded **up to a multiple of 64** so each worker owns whole `u64`
validity words and no two workers touch the same word. The nested-ceiling identity
`ceil(ceil(n/64) / (c/64)) == ceil(n/c)` for `64 | c` guarantees the value-chunk
and word-chunk iterators yield equal counts, so the `zip` drops no work.

Bit-identity argument: `f` is unchanged and per-index; the NaN ⇒ invalid rule is
per-index; `all_valid` and `all_finite` are boolean AND reductions. So neither the
values, the mask, nor the witness depends on how the range is split.

## FP-side measurement (interleaved, with A/A null control)

Two immutable ELFs, both built strict-remote on `vmi1227854` from the exact base
above, differing only in `crates/fp-columnar/src/lib.rs`:

| Arm | sha256 | bytes | `par_map_vec_f64_with_witness` symbols |
|---|---|---:|---:|
| reference (`--no-overlay`) | `a1cbe86ef629aa1711eec37f067fba1c55ba862d76e2dd1f582f3110e7213fe6` | 75,301,280 | **0** |
| candidate (`--overlay-path crates/fp-columnar/src/lib.rs`) | `53050cb12f4d531d619b24146cf019ff2feadf6d86ae04e34be036589a8ef38a` | 75,340,360 | **138** |

Candidate and reference invocations were interleaved within one time window,
order alternated per round, with an A/A control running the *same* candidate
binary twice at the same cadence to establish the noise floor.

| workload, 1M Float64 | reference p50 | candidate p50 | self-speedup | bootstrap 95% CI | A/A null | A/A 95% CI | separated? |
|---|---:|---:|---:|---:|---:|---:|---|
| `math_unary/sqrt` | 2259.4 us | 1479.8 us | **1.5268x** | [1.4787, 1.6029] | 0.9963x | [0.9430, 1.0556] | YES |
| `math_unary/log` | 2520.0 us | 1869.1 us | **1.3482x** | [1.2695, 1.4227] | 1.0148x | [0.9649, 1.0654] | YES |

Both effect CIs lie entirely above their A/A null CIs. Absolute times are
inflated by peer load and must not be quoted as quiescent-host figures; the
interleaving is what makes the *ratio* meaningful under drift.

**Output identity:** the harness checksum was `e700f53534db5c6d` on **every arm of
every round of both workloads** — reference and candidate, sqrt and log.

## Semantic evidence

- `cargo test -p fp-columnar --profile release-perf`: **595 passed, 0 failed**, 57 ignored.
- New test `fused_par_witness_matches_scalar_reference_across_chunk_boundaries`
  compares `sqrt` and `log` bit-for-bit (`f64::to_bits`) against the ops' Scalar
  reference path at n = 199,999 / 200,003 / 262,145 / 393,281 — lengths chosen to
  straddle the 200,000-row parallel threshold and to avoid multiples of 64 and of
  the worker count, so both a ragged final chunk and a ragged final validity word
  are exercised. Input deliberately poisons 64-aligned boundary slots with
  negatives (⇒ NaN ⇒ missing), NaN payloads on valid bits, and input gaps. A
  final all-valid, NaN-free 300,007-row case covers the `all_valid` branch.

**This test closed a real coverage hole.** Every pre-existing nullable-unary test
in the file draws `n = next() % 200`, i.e. below the 200,000-row threshold, so
before this change *the entire chunked parallel path had zero coverage*.

**Negative control (the test has teeth):** replacing
`n.div_ceil(workers).div_ceil(64) * 64` with `n.div_ceil(workers)` — removing only
the 64-alignment — makes the new test fail immediately on all eight workers with
an index panic at the word write. Restored and re-verified green afterwards.

## Blocked measurement, disclosed rather than worked around

No live-pandas ratio is claimed because the harness's host-wide exclusivity gate
(`MAX_HOST_WIDE_BUSY_FRACTION = 0.20` across **every online CPU**, two consecutive
clear 1 s samples, re-adjudicated per phase, fail-closed) never cleared:

- **`thinkstation1`**: blocked at `phase=invocation_preflight` after 20 attempts.
  Persistent offenders were cross-project peers — a franken-networkx python job
  pinned at ~104%, frankensearch `rustc`/`parallel_shard_ingest_ab --bench`, and
  frankenredis `redis-benchmark`. None are this agent's to kill.
- **`threadripperje`**: sampled 8 consecutive seconds, **0/8 clear**, with three
  CPUs pinned at 1.00 (an `asupersync` `git fsck` and a peer python job).

Useful finding for successors: **`thinkstation1` already satisfies the harness's
exact incumbent pins with no setup** — `--dependency-probe` reports
`pandas 2.2.3 / pyarrow 24.0.0` and `--host-exclusivity-self-test` passes. The
drain-an-rch-worker + `pip --target` route is therefore not the only option; the
workstation needs no pandas install, only quiescence. By contrast `threadripperje`
has **no pandas on its default interpreter** and needs `PYTHONPATH` (staged trees
exist at `/tmp/fpbench-libs-h67zz` and `/data/tmp/fpbench-libs`).

Also confirmed by reading the gate: `expected_cpu_ids` enumerates
`/sys/devices/system/cpu/*/online`, so its scope is genuinely
`all_online_host_cpus`. Running the benchmark under a narrow `taskset` does **not**
narrow the gate — an affinity note in an artifact is thread provenance, not a
weakened instrument.

## What this does and does not settle

The fusion removes a serial pass that was up to a third of the operation. It does
**not**, on its own, establish a win against pandas: the pre-change default-target
`sqrt` was 0.250x and v3 was 0.361x, so a ~1.5x FP-side gain narrows that gap
substantially without any measurement showing it crossing 1.0. **No projection of
the post-change pandas ratio is offered or implied.** That row stays open until a
quiet host is available.

## Named next lever (mechanism proven, deliberately NOT bundled)

`from_f64_values_owned` (`crates/fp-columnar/src/lib.rs:9513`) opens with
`if data.iter().any(|v| v.is_nan())` — the identical serial full-output scan, and
on a NaN hit `from_f64_values` scans the buffer twice more. It is the output
constructor for `typed_float_unary_par`, which serves **17 further ops**: `log10`,
`log2`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `sinh`, `cosh`, `tanh`,
`asinh`, `acosh`, `atanh`, `exp_m1`, `ln_1p`, `cbrt`.

It was left out of this change so the A/B above stays attributable to one edit.
Note that **none of those 17 ops currently has a `run_math_unary` bench workload**,
so landing that widening under this project's measure-before-keep rule requires
adding at least one (e.g. `log10`) to `crates/fp-bench/src/main.rs` first.
