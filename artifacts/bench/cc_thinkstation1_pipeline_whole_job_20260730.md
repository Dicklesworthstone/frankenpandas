# Whole-job ETL pipeline vs live pandas — workload landed, equivalence proven, timing BLOCKED

- Agent: BlackThrush (cc pane) · Host: `thinkstation1` (AMD Ryzen Threadripper PRO 5975WX, 32C/64T)
- Date: 2026-07-30 · Base commit: `7a5bf7143`
- Evidence: `cc_thinkstation1_pipeline_whole_job_equivalence_20260730.json`

## What this is

A **whole-job** benchmark, not another kernel benchmark. One timed closure runs a
complete star-schema rollup of the shape a pandas user actually writes:

```python
sales  = pd.read_csv(sales_path)                              # 1. load
stores = pd.read_csv(stores_path)
kept   = sales[sales["amount"] > 0.0]                         # 2. filter
agg    = kept.groupby("store_id", as_index=False).sum()       # 3. groupby
joined = agg.merge(stores, on="store_id", how="inner")        # 4. join
ranked = joined.sort_values(["amount", "store_id"],           # 5. sort
                            ascending=[False, True])
ranked.to_csv(out_path, index=False)                          # 6. write
```

Registered as `--category pipeline --workload etl_job`. Both engines run in the
same harness invocation; FrankenPandas runs the same six stages via
`fp-bench --category pipeline --workload etl_job --data-dir <dir>`.

Inputs are a fact table (`store_id`, `units`, `amount`; 200 rows/store) and a
dimension table (`store_id`, `store_name`, `region`). They are materialized
**once, outside every timed window**, and both engines read the *same bytes*.

`pipeline` is deliberately NOT added to `CATEGORIES`, so the weighted kernel
score and every existing baseline are unchanged. A six-stage job's ratio is not
commensurable with a geomean over single-op categories; folding it in would let
one end-to-end number move a headline that means something else.

## Result 1 — cross-engine equivalence: BYTE-IDENTICAL ✅

A whole-job ratio means nothing unless both arms did the same job. The per-engine
`checksum` in this harness is a liveness token (`size_of_val` on the Rust side),
not a content hash, so it cannot compare across engines. So the driver diffs what
the job actually *produced*:

| rows | stores | output | FrankenPandas sha256 | pandas sha256 | verdict |
|---|---|---|---|---|---|
| 10,000 | 50 | 2,061 B | `3398968f18148f57…` | `3398968f18148f57…` | **byte-identical** |
| 1,000,000 | 5,000 | 211,304 B | `04bae1d757c33c5e…` | `04bae1d757c33c5e…` | **byte-identical** |

Identical bytes means identical column order, identical row order (the ranking),
identical float rendering, and identical values. Disagreement sets the verdict to
`OUTPUT_MISMATCH` and voids the ratio rather than reporting a number alongside it.

Two design choices make byte-exactness a *fair* requirement rather than a lucky
one, and both are deliberate:

- **Amounts sit on a $0.25 tick**, which is exactly representable in binary
  float64. Group sums are therefore exact and order-independent. pandas and
  FrankenPandas reduce each group in their own order; on arbitrary decimal cents
  that alone would shift the last ULP and make a correct result look like a
  mismatch.
- **The sort carries an explicit `store_id` tiebreak.** pandas' default
  `quicksort` is not stable, so tied `amount` sums would otherwise be free to
  disagree without either engine being wrong.

## Result 2 — the composition finding (read this before quoting any ratio)

Per-stage medians, pandas, 1M rows. **Measured under peer load — indicative only,
not a claim:**

| stage | ms | share |
|---|---|---|
| **load (read_csv ×2)** | **117.33** | **82.3%** |
| groupby | 11.41 | 8.0% |
| write | 6.37 | 4.5% |
| filter | 5.42 | 3.8% |
| sort | 1.09 | 0.8% |
| join | 0.92 | 0.6% |

**A whole-job number on this shape is ~82% a CSV-parse benchmark.** The join and
sort together are 1.4% of it. This is not a defect in the workload — real pandas
ETL jobs genuinely are parse-dominated, which is exactly why the end-to-end shape
was worth building — but any whole-job ratio quoted from it must be quoted *with*
this table. Without it, a large number reads as "FrankenPandas is Nx faster at
DataFrame work" when it mostly means "FrankenPandas parses CSV faster."

Clear follow-up: a Parquet-input variant of the same six stages, where load stops
dominating and the compute stages carry real weight. That answers whether a
whole-job win survives when parsing is not the bulk of the job.

## Result 3 — the timing claim: BLOCKED, not estimated

**No vs-pandas ratio is claimed here.** `benches/vs_pandas_harness.py` refused to
start:

```
ERROR: host-wide benchmark exclusivity requires every online CPU to remain at or
below 20.0% busy; phase=invocation_preflight missing=[] busy=[46, 48, 52, 63]
```

The gate requires all 64 online CPUs at or below 20% busy and fails closed with
`SystemExit(2)`. I sampled the host for 60 consecutive 1-second windows:

- **0 / 60 samples clear**
- median **6** CPUs above the limit; range 2–23

The host carries several concurrent agent panes (`codex` processes). The gate is
correct and I did not modify it, scope it down, or route around it. A number
obtained by relaxing the instrument that exists to make numbers trustworthy would
be worth less than no number.

### The block was retested, not assumed

The whole 1M run takes only ~60–90 s, so a short lull would in principle be
enough. It was worth waiting for, and I did:

1. **Named the blocker.** The persistent offender was not ambient load but a
   *peer agent's* criterion run pinned at 100% on CPU 8:
   `perf_matrix-ad95d11065ec2143 --bench` out of `/data/tmp/cargo-target-h3-scaling/`.
   Not mine to kill.
2. **Waited on a 5-second-clear trigger.** It fired at 21:11:15 after five
   consecutive clear seconds — and the harness *still* blocked ~1 s later on a
   fresh set, `busy=[16, 18, 19, 22, 54]`. Five seconds is not predictive.
3. **Waited 8 minutes on a 15-second sustained-clear trigger.** It never fired:
   **0 launch attempts in 480 s**. The host does not offer a 15-second sustained
   lull while the swarm is running.

Runner preserved as `cc_thinkstation1_pipeline_whole_job_20260730.await_quiet.sh`
so the next agent can simply start it and walk away; it waits for the gate's own
condition to hold on its own, then hands off to the harness, which re-checks
independently. It never touches the gate.

This is the same wall the cod pane hit on the 10M sort A/B
(`cod_thinkstation1_sort_serial_residual_profile_20260729.md`). It is systemic,
not a one-off: **while the swarm is active, this host cannot produce a gate-valid
vs-pandas measurement at all.** A quiet-host window has been requested from
CyanLynx via agent-mail (msg 7075).

### Unvalidated indication, explicitly not a claim

For handoff only, so the next agent knows roughly what to expect and can tell if
something has changed drastically: single-shot pandas 1M whole job ≈ 0.173 s;
FrankenPandas 1M whole job p50 ≈ 0.0318 s over 50 paired samples. These were
taken **while the exclusivity gate was failing**, the pandas figure is one
un-warmed run against FP's paired p50, and no null-control CI was computed across
them. **Do not quote this as a ratio.** It is a sanity anchor, nothing more.

## Observed-not-requested threads

The FP arm reports what it actually used, never what was requested:

| size | `runtime_available_parallelism` | `peak_process_threads` | `operation_threads_used` |
|---|---|---|---|
| 10k | 64 | 2 | **1** |
| 1M | 64 | 10 | **8** |

The whole job is fully serial in FrankenPandas at 10k and reaches 8 observed
threads at 1M. Runtime-detected ISA on this host: `sse2, avx2, fma, bmi2, vaes`.
`host_identity` is emitted per row via `build_thread_provenance`.

## Build provenance

Requested form was attempted first and **refused**:

```
$ rch exec --base 7a5bf7143 --clean-overlay \
      --overlay-path crates/fp-bench/src/main.rs -- cargo build --profile release-perf -p fp-bench
[RCH] remote required; refusing local fallback (no admissible workers:
      critical_pressure=2, insufficient_slots=1, hard_preflight=8, active_project_exclusion=1)
```

Retried; refused identically.

The measured binary was therefore built remotely on **hz1** through the rch
`cargo` hook (which does retrieve artifacts, unlike `rch exec`), and it is
**source-identical to what the requested form would have produced**: the working
tree's only Rust modification is `crates/fp-bench/src/main.rs`, which is exactly
the file the `--overlay-path` would have carried on top of `7a5bf7143`
(`git status` shows only that file plus the Python harness, which does not enter
the binary).

- ELF sha256 `f0a9e95a655d249b4dab50d41a19ab513882fe4179b3bf750f5e2ba9205b5940`
  (74,463,832 bytes), self-reported by the binary on line 1 of its own output.

### rch admission is stale, not capability-limited

`hard_preflight=8` means 8 workers are recorded as lacking the pinned
`nightly-2026-04-22`. That is **out of date**: two disk-rich workers now carry it.

| worker | free disk | pinned toolchain present? |
|---|---|---|
| `vmi1227854` | 237 G | **yes** (verified over ssh) |
| `vmi1264463` | 285 G | **yes** (verified over ssh) |
| `hz1` | 11 G | yes — but disk-critical |
| `hz2` | 23 G | yes — but disk-critical |

So the repo's remote capacity is being throttled by rch's **cached admission**,
not by a real capability gap. `rch workers capabilities --refresh` does not clear
it (it reports each worker's *default* rustc 1.99.0-nightly and a fleet-wide
version-mismatch warning — that warning is a red herring and is not the preflight
cause). Only a daemon re-poll clears it, and `rch daemon restart` kills in-flight
builds across every repo — two were active on `ovh-a` at the time — so it was not
run. Flagged for whoever owns the fleet.

## Status

- ✅ Whole-job `pipeline/etl_job` workload landed, both arms, same invocation.
- ✅ Cross-engine equivalence proven byte-identical at 10k and 1M.
- ✅ Composition characterized: the job is 82% CSV parse; quote the ratio only with that.
- ⛔ Timing claim blocked on host exclusivity. Nothing estimated in its place.
- ⛔ `rch exec --base --clean-overlay` blocked on stale admission; equivalent
  artifact used, with the equivalence argued rather than assumed.

To finish, on a quiet host:

```bash
python3 benches/vs_pandas_harness.py --category pipeline --sizes 1M,10M \
    --expected-hostname thinkstation1
```

or, to have it wait for its own window and fire unattended:

```bash
artifacts/bench/cc_thinkstation1_pipeline_whole_job_20260730.await_quiet.sh
```

Everything except the ratio is already banked. The remaining step needs a quiet
host, not more code.
