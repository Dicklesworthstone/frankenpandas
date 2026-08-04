# br-frankenpandas-uza04.117 — Float64 value_counts all-tie sort guard — rejected

Agent: LavenderStone
Date: 2026-06-13

## Target

Post `br-frankenpandas-8sxez`, the Float64 `Series::value_counts` 100k
workload still costs about 9-11 ms in the direct `fp-bench` timed window. The
prior profile-backed `uza04.103` proof identified the residual after Float64
bit-tally as label materialization plus the stable count sort. `8sxez` removed
the textual Float64 index-label materialization, leaving the stable sort as the
next plausible local lever.

Kernel/user sampling was not available in this environment:

- `perf record`: blocked by `perf_event_paranoid=4`, status 255.
- `samply record`: blocked by `perf_event_paranoid=4`, status 1.

Those blocked profiler transcripts are preserved in:

- `tests/artifacts/perf/uza04117_base_perf_record_value_counts_100k_float64.txt`
- `tests/artifacts/perf/uza04117_base_samply_record_value_counts_100k_float64.txt`

## Baseline

Build:

- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavenderstone-uza04117-base rch exec -- cargo build -p fp-bench --profile release-perf --bin fp-bench`
- RCH fell open locally because no admissible worker slots were available.

Direct `fp-bench --category dataframe_ops --workload value_counts --size 100k --dtype float64 --json`:

- mean: 10.802 ms
- p50: 10.363 ms
- artifact: `tests/artifacts/perf/uza04117_base_fp_bench_value_counts_100k_float64.json`

Hyperfine full-process:

- 300.1 ms +/- 9.0 ms
- artifact: `tests/artifacts/perf/uza04117_base_hyperfine_fp_bench_value_counts_100k_float64.txt`

## Candidate Lever

Guard the default `Series::value_counts` stable descending sort with an
all-counts-equal check. When every bucket has the same count, the stable sort is
a semantic no-op: pandas tie-breaking keeps first-seen order, and the existing
count vector is already in first-seen order.

Behavior proof for the candidate:

- Ordering: unchanged for non-all-tie counts because the same stable count sort
  runs; unchanged for all-tie counts because stable sorting by equal keys returns
  the input first-seen order.
- Tie-breaking: preserved exactly by first-seen order.
- Floating-point/RNG/hash behavior: no arithmetic, RNG, or hash-key change.
- Null/NaN: default `value_counts` missing handling was untouched.

## Result

Build:

- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavenderstone-uza04117-after rch exec -- cargo build -p fp-bench --profile release-perf --bin fp-bench`
- RCH worker: `vmi1227854`.

Direct same-command timing:

- baseline mean: 10.802 ms
- candidate mean: 9.672 ms
- speedup mean: 1.117x
- baseline p50: 10.363 ms
- candidate p50: 9.433 ms
- speedup p50: 1.099x
- artifact: `tests/artifacts/perf/uza04117_fp_bench_value_counts_delta.json`

Hyperfine full-process:

- baseline: 300.1 ms +/- 9.0 ms
- candidate: 305.1 ms +/- 12.7 ms
- speedup mean: 0.984x
- artifact: `tests/artifacts/perf/uza04117_hyperfine_value_counts_delta.json`

## Decision

Rejected. The direct timed window improved only about 1.12x, while the
full-process hyperfine gate was slightly slower/noisy. Score is below 2.0, so no
source code is retained.

The source hunk and its focused test were removed before commit. This proof and
the timing artifacts remain to prevent repeating the same micro-lever.

## Next Route

Do not repeat the all-tie sort guard. Attack a deeper Float64 value_counts
primitive instead: keep the tally/output path typed through materialization
(`u64` key + first-seen `f64` + count) and construct `IndexLabel::Float64` plus
`Scalar::Int64` output directly, avoiding the residual `(Scalar, usize)` vector
and second-pass scalar remapping while preserving exact key canonicalization,
first-seen order, stable count ordering, null dropping, and Float64 bit payloads.
