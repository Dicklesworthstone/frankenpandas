# Row-morsel parallel filter gather — REJECT (0.9488x, measured slower)

**Agent:** cod-pandas (claude-code / opus-5)
**Date:** 2026-08-02
**Host:** `thinkstation1`, AMD Ryzen Threadripper PRO 5975WX, 32 physical / 64 logical
**Base commit:** `1087e10d3`
**Decision: REVERT.** The change is correct and routes as designed, and it is
**5.1% slower** on the whole job. Reverted; the working tree is back to base.

## Classification (read before quoting any number)

FP-side self-comparison under peer load. **Maintenance-class evidence, NOT a
vs-pandas claim.** The host-wide quiescence gate was never satisfiable during
this session (51–55 of 64 CPUs above the 20% limit throughout, from a peer's
`frankensearch_quill_gauntlet`, a `headtohead` run at 910%, and a redis
benchmark). The gate was **not relaxed, scoped down, or routed around**. Since
the verdict here is a REJECT, a gate-valid window would not change it: a change
that loses to its own predecessor cannot win against pandas.

## The lever and why it was chosen

The whole-job ETL profile's largest remaining cluster was the boolean filter.
`take_rows_by_positions_with_affine_certificate_unchecked` fans its gather out
**one column per worker**:

```rust
let worker_count = available_parallelism()... .min(64).min(ncols.max(1));
```

so the canonical three-column filter used **3 of 64 cores** and streamed the
entire positions tape once per column. Column count, not the machine, set the
width. The lever: split by ROW morsel instead, so every worker owns a row range
in all columns, width scales with cores, and each morsel's tape slice is read
once into cache instead of `ncols` times from memory. A single-column filter
would parallelize too, where the per-column fan-out can only ever use one worker.

Implemented as `Column::take_positions_many` in fp-columnar (so it could
reproduce the private per-dtype backings exactly), called from the frame filter
path ahead of the existing fan-out.

## It worked. It was still slower.

Interleaved A/B, order alternated every round, A/A null control (baseline run
twice per round), 12 rounds, `pipeline/etl_job_parquet` at 1M rows:

| arm | p50 | n |
|---|---:|---:|
| baseline (`2e45d6a1…`, 75,881,992 B) | 24,512.6 us | 12 |
| candidate (`af98f20e…`, 75,954,672 B) | 25,836.7 us | 12 |

- **Effect, baseline/candidate: 0.9488x**, 95% bootstrap CI **[0.9337, 0.9652]** — excludes 1.0.
- **A/A null control: 0.9978x**, CI **[0.9903, 1.0133]** — straddles 1.0, so the instrument is sound.
- **All 12 interleaved round ratios are below 1.0** (0.8820 – 0.9801).
- Operation threads: baseline 4, candidate 9.

## Why — the profile says it plainly

`perf record -F 999 --all-user`, same host, same inputs, both arms:

| symbol | baseline | candidate | reading |
|---|---:|---:|---|
| `__memset_avx2_unaligned_erms` | 13.67% | **20.94%** | **+7.3 pts — the cost** |
| `Column::take_positions` | 6.85% | 0.07% | the old serial-per-column gather, gone |
| `take_positions_many` worker closure | — | 3.23% | the new gather, ~half the cost |
| `loc_bool_with_affine_witness` | 18.82% | 17.73% | unchanged (the tape build) |

The gather itself got **cheaper by ~3.6 points, exactly as designed**. It was
overwhelmed by **+7.3 points of memset**.

**Root cause.** A parallel scatter needs the output buffer to exist before the
workers can be handed disjoint `&mut` chunks of it, so the row-morsel path must
`vec![0.0; rows]` each output column — three buffers, ~11.9 MB at this shape —
and then overwrite **100%** of the bytes it just zeroed. The per-column path it
replaced used `Vec::with_capacity` + `push` and never zeroed anything. This is
the tax already recorded in [[parallel-per-call-overhead-ledger]] (`vec![0.0;
1M]` ≈ 206 us, fully overwritten), now measured at whole-job scale.

## The generalizable finding

**Under `#![forbid(unsafe_code)]`, converting a serial push-based producer into
a parallel scatter costs a full zero-fill of its output. For a memory-bound
producer that exceeds the parallel saving.**

There is no safe way out from inside the kernel: handing workers uninitialized
output chunks needs `spare_capacity_mut` + `set_len`, i.e. `unsafe`. Every safe
alternative moves the cost rather than removing it — per-worker `Vec`s plus a
concatenation trade an 11.9 MB zero-fill for an 11.9 MB serial memcpy.

This retroactively explains why the house helper `par_map_vec_f64` caps at 8
workers, and why the elementwise-map vectorization landed only ~1.07x.

**It also predicts the next lever down.** Parallelizing `boolean_mask_positions`
(the 18.82% serial mask→positions compaction, the baseline's #1 entry) via
count-then-scatter would need a preallocated `vec![0usize; kept]` and would pay
this same tax. **Do not run that experiment expecting a different answer.** The
directions this evidence actually favours:

1. **Fusion that removes the materialization entirely** rather than parallelizing
   it. `sales[mask].groupby(k).sum()` forces pandas to materialize `kept` — its
   API leaves it no choice. FrankenPandas has a choice. Feeding the groupby fold
   through the positions tape means the filtered value buffers are never
   allocated, so there is no output to zero and no gather to parallelize. This is
   the one direction where the memset finding is a tailwind.
2. **Escalate the `unsafe` policy question with a price attached.** This is no
   longer abstract: the zero-fill is worth ~5% of a whole ETL job on this shape,
   and it gates an entire family of scatter levers. That is a decision for the
   owner, not something to quietly work around.

## Correctness (established before the timing, and it held)

The change was not rejected for being wrong. It was verified first:

- **Byte-identical whole-job output.** Both arms produced
  `04bae1d757c33c5e2eae36ccb00c3bfb3732e4d10632b499b0ebf845c63f35ce` — which is
  the same sha256 `cc_thinkstation1_pipeline_whole_job_20260730.md` recorded for
  **pandas** at 1M, so equivalence holds against the incumbent, not just against
  the previous FP build. Per-engine `checksum` also matched (`a322e43db0997d7d`).
- `cargo test -p fp-columnar --lib`: **600 passed, 0 failed**, 57 ignored.
- `cargo test -p fp-frame --lib`: **3191 passed, 0 failed**, 26 ignored.
- Four new tests, each carrying a **routing assertion** so none could pass
  vacuously — including one at 150k rows, because every pre-existing filter test
  is a handful of rows and would never reach the 2^18 admission threshold. That
  gap is the one that previously let a parallel elementwise path ship with zero
  coverage.
- Byte-identity was structural, not hoped-for: the path declined any ascending
  arithmetic progression, which is exactly the shape that routes a Float64 column
  to `take_positions`'s zero-copy contiguous/strided views, so it provably could
  not shadow a view with a copy.

## Measurement hygiene note — a confound that was caught and removed

A peer agent was editing `crates/fp-columnar/src/lib.rs` concurrently in this
shared checkout (their hunks reroute `radix_argsort_multi_u64` to a new parallel
implementation). A working-tree build would have measured **their change and
this one as a single effect**. The candidate was therefore built from pristine
HEAD plus only this session's three hunks; verified by symbol count — candidate
carries `take_positions_many` (7 symbols) and **zero** `parallel_radix_argsort_multi_u64`,
baseline carries neither. The revert reverse-applied only this session's hunks;
the peer's uncommitted work is untouched.

## Reproduction

```
# inputs from the harness's own generator, outside every timed window
python3 -c "...materialize_pipeline_inputs_parquet(1_000_000, d)"
python3 interleaved_ab.py fp-bench-baseline fp-bench-candidate etl_inputs 12
```

Scratch (A/B driver, hunk splitter, both ELFs, perf data) retained under
`/data/tmp/claude-1000/-data-projects-frankenpandas/7364bdd3-.../scratchpad`.
