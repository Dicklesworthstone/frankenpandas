# Whole-job ETL profile → the `as_index=False` dense-groupby lever

**Agent:** SwiftHill (claude-code / opus-5)
**Date:** 2026-07-31
**Host:** `thinkstation1`, AMD Ryzen Threadripper PRO 5975WX, 32 physical / 64 logical
**Base commit:** `f00b8da8016c569ccd02bbedc1dc8a715d265dc5`

Decision: **KEEP.** Routing proof obtained and the whole job is **1.4373x** faster
FP-side. No vs-pandas ratio is claimed for the whole job — that row is still open.

## Routing proof (the claim this lever actually rested on)

The shape-invariance test proves the two paths agree; it would pass even if the
bypass never fired. The decisive evidence is the re-profile, same workload, same
inputs, candidate ELF `f0c89e0e…` vs reference `7aef9c95…`:

| symbol | reference | candidate |
|---|---:|---:|
| `DataFrameGroupBy::build_groups` | 14.2% (**#1 entry**) | **absent** |
| `DataFrameGroupBy::aggregate_named_func` | 10.0% | **absent** |
| `DataFrameGroupBy::format_output` | 1.5% | **absent** |
| `DataFrameGroupBy::int64_dense_grouping` | — | **2.11%** (new) |
| `DataFrameGroupBy::dense_aggregate_emit` | — | **2.62%** (new) |
| `OnceLock<Vec<Scalar>>::call_once_force` | 3.9% | 2.7% |

The generic hash-grouping path is gone and the dense bypass is in its place: the
groupby cluster fell from **~25.7% to ~4.7%** of whole-job self time. The residual
leaders are now exactly the shared costs ruled out above (filter gather, Parquet
decode, memset/memmove), whose *relative* shares rise precisely because the total
shrank.

## Whole-job measurement (interleaved, A/A null control)

Candidate and reference interleaved within one window, order alternated per round,
plus an A/A control running the candidate twice at the same cadence.

| arm | p50 | n |
|---|---:|---:|
| reference (pre-change) | 40,162.4 us | 10 |
| candidate | **27,942.5 us** | 10 |

- **Whole-job self-speedup 1.4373x**, bootstrap 95% CI **[1.2273, 1.7897]**.
- A/A null control **1.0002x**, CI **[0.8456, 1.1800]**.
- Separated — but the margin is narrow (1.2273 vs 1.1800) and the A/A interval is
  wide because the host was not quiet. Treat 1.4373x as directional, not as a
  gate-admitted figure. Operation threads observed: 3–5.

This is a **whole-job** number on a six-stage ETL job, from a single routing fix —
not a kernel microbenchmark.

⚠️ Not claimed: any vs-pandas whole-job ratio. That needs the host-wide gate.

## Method: profile the whole job, then ask who else pays

Rather than profile a kernel, this profiles the most realistic end-to-end workload
in the tree — `pipeline/etl_job_parquet`, the six-stage star-schema rollup
(load → filter → groupby → join → sort → write) whose output is already proven
**byte-identical** to pandas at 10k and 1M. The Parquet variant is the right one
to profile: `etl_job` at 1M is **82.3% `read_csv` on the pandas side**, so its
whole-job ratio is mostly a CSV-parse ratio, whereas the Parquet variant has cheap
load and lets the compute stages carry the weight.

Inputs were reproduced exactly by importing the harness's own
`materialize_pipeline_inputs_parquet`, so both engines read the same values.

`perf record -F 999 --all-user`, 5,181 samples; a second pass with
`--call-graph dwarf,16384` for attribution. FP whole-job p50 was 42.6 ms at 1M,
using **3 operation threads** on a 64-core host.

### Top self-time, each judged by "does the incumbent pay this too?"

| entry | self | does pandas pay it? | verdict |
|---|---:|---|---|
| `DataFrameGroupBy::build_groups` | 14.2% (17.0% dwarf) | only because of **our** gate | **structural — the lever** |
| `Column::take_positions` | 10.3% | yes — boolean indexing copies | shared, move on |
| `DataFrame::loc_bool_with_affine_witness` | 10.1% | yes — same | shared, move on |
| `DataFrameGroupBy::aggregate_named_func` | 10.0% | partly — see below | mixed |
| `fp_io::read_parquet_bytes` | 7.1% | yes — pyarrow decodes the same file | shared, move on |
| `__memset_avx2_unaligned_erms` | 6.9% | yes — allocates constantly | shared, move on |
| `RleDecoder::get_batch_with_dict::<i64>` | 4.9% | yes — same Parquet encoding | shared, move on |
| `__memmove_avx_unaligned_erms` | 4.6% | yes — copies constantly | shared, move on |
| `OnceLock<Vec<Scalar>>::call_once_force` | 3.9% | **no equivalent exists** | **structural** |
| `drop_in_place::<ScalarValues>` | 3.2% | **no equivalent exists** | **structural** |

The memset/memmove rows are the trap this method exists to avoid: they are large,
they look actionable, and they are simply what any engine pays to allocate and
copy. pandas pays them too, so they are not the gap.

## The structural difference

DWARF stacks resolved the `Vec<Scalar>` forcing to
`fp_columnar::ScalarValues::as_slice` → `OnceLock<Vec<Scalar>>::get_or_init` →
`collect` **from a `Copied<slice::Iter<i64>>`** — an Int64 column that *already
owns a contiguous `&[i64]`* being expanded into one boxed enum per row. The
callers were `DataFrameGroupBy::aggregate_named_func` and
`DataFrameGroupBy::format_output`.

Root cause, `crates/fp-frame/src/lib.rs`: the single-key Int64 **dense bypass** —
hash-free grouping that skips `build_groups` entirely — opened with

```rust
if self.as_index
    && self.by.len() == 1
    && matches!(func_name, "sum" | "mean" | ...)
```

`as_index` selects only the **output shape**: whether the grouping key is returned
as the index or as a leading regular column. It cannot change which groups exist
or what the aggregation computes. Gating a *compute* fast path on an *output-shape*
flag stranded `groupby(key, as_index=False).sum()` — the canonical ETL idiom, and
the one this pipeline uses — on the generic path, which then pays twice:

1. `build_groups` — a full hash-grouping pass, the #1 self-time entry; and
2. `format_output`'s `as_index=False` branch does `src_col.values()[first_row]`,
   forcing a **full `Vec<Scalar>` materialization of the 1M-row key column** just
   to pick one representative per group (5,000 of them).

**pandas has no analogue for either.** `as_index=False` there is a post-step, and
its groupby hashes a numpy int64 array directly — there is no boxed-Scalar layer
to force or drop. That is what makes this a structural difference rather than a
constant-factor race.

## The change

Drop `self.as_index` from the bypass gate, and have `aggregate_int64_dense`
reshape when it is false. The reshape is exactly `DataFrame::reset_index(false)`,
which already puts the index back as the FIRST column over a default integer
range and has a typed `int64_label_values` path — so the key returns to a column
with **no Scalar round trip**, and the reshape touches `ng` output rows rather
than the input.

## Evidence so far

- `cargo test -p fp-frame --lib`: **3190 passed, 0 failed**, 26 ignored.
- Targeted groupby subset: **212 passed, 0 failed**.
- New test `groupby_as_index_false_int64_dense_bypass_changes_shape_only` pins the
  invariant the change rests on: for a bounded Int64 key with dense Int64+Float64
  values, every aggregated column must be identical between the `as_index=True`
  and `as_index=False` arms; only the key's placement may differ.

⚠️ **That test proves correctness, NOT routing.** It would pass even if the bypass
never fired, because both paths are supposed to agree. The decisive routing proof
is `build_groups` disappearing from a re-profile of the candidate, which is also
the measurement. **It has not been obtained and no speedup is claimed.**

### Pre-existing property, noted rather than introduced

`reset_index` builds through `DataFrame::new_with_axes`, which defaults
`allows_duplicate_labels`, whereas the generic `format_output` propagates
`self.df.allows_duplicate_labels`. `dense_aggregate_emit` already used
`new_with_axes`, so the dense family has always defaulted that flag for
`as_index=True`. This change extends existing dense-path behaviour; it does not
create a new discrepancy.

## Measurement status — disclosed, not worked around

The vs-pandas gate requires two consecutive samples with **every** online CPU at
or below 20% and re-adjudicates **per phase**. Over ~3 hours two independent
watchers recorded windows, but they are **shorter than one six-phase run**: a
retry loop cleared `invocation_preflight` *and* `post_provenance` with
`busy_cpu_count_above_limit=0` across all 64 CPUs, then failed at
`pre_measurement:pandas:math_unary/sqrt/1M/float64`. Host load oscillated between
8 and 150 during the session, and a peer's cleanup sweep killed three of this
agent's jobs mid-run at the peak.

That is a sharper diagnosis than "blocked": windows exist and are sub-30-second,
so the instrument is not broken and must not be relaxed — it needs a cheap,
always-waiting retry loop, which is what is running.
