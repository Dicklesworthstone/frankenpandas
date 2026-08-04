# br-frankenpandas-uza04.114 - Kendall rank-signature route rejection

Status: rejected before runtime source edit; no code retained.

## Target

`br-frankenpandas-uza04.114` was opened after no ready unclaimed `[perf]`
child bead appeared and `br-frankenpandas-0dm7c` pointed at exact all-pairs
Kendall rank-signature / offline-dominance sharing as the deeper route.

Fresh inspection showed that this exact route had already been explored in the
current campaign:

- `uza04.93`: fixed-left CDQ target-panel row builder, exact but slower.
- `uza04.94`: bitplane prefix witness, exact but slower.
- `uza04.95`: row-pair sign-tensor formulation rejected before source edit
  because exact aggregation collapses to explicit row-pair enumeration or the
  already rejected dominance formulations.
- `uza04.99`: kept the actual shifted Kendall/Spearman prep bottleneck by
  parallelizing per-column Kendall prep and using lazy shared row indexes.

## Fresh Baseline

Baseline binary:

`/data/projects/.scratch/cargo-target-lavenderstone-uza04114-base/release-perf/examples/perf_profile`

Build command:

`CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavenderstone-uza04114-base rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`

RCH result: failed open locally because no worker was admissible
(`insufficient_slots=2,hard_preflight=10`), so the fresh timings are routing
evidence only and not acceptable keep/reject proof for a future source lever.

Golden output SHA256:

- `df_kendall 2000`: `acf366c266b66f8497fb55734ed1b4ec40952a7c014f43db262c7c1e625e15e1`
- `df_kendall 5000`: `031978ba431260b942dd36d9be055064a7453118a7c00888c35747644d33d99e`
- `df_kendall 20000`: `f164fa86300fbea93e46aa99a5e0f7413fa8c0baf2ee49c6a689ed92652a4c3b`

Fresh local fail-open hyperfine:

- `df_kendall 50000x1`: `49.8 ms +/- 20.0 ms` (noisy; outliers reported)
- `df_kendall 200000x1`: `178.5 ms +/- 5.1 ms`

`perf stat` was blocked by host policy:

`perf_event_paranoid setting is 4`

## Decision

Reject this route before editing source. This is not a conclusion that Kendall
has no remaining wins; it is a non-repeat guard for a route that is already
covered by stronger current artifacts. The next productive attack should move
to a different profile-backed primitive instead of reopening CDQ, bitplane,
row-pair sign tensor, static rank/select, row-major rank-signature batching,
dynamic multi-Fenwick batching, merge-sort, sqrt/block counters, validation
hoists, buffer reuse, or cache-layout micro-tuning.

Next target: the df-wide numerical/output residuals already identified by the
campaign artifacts, especially `df_dot` output-allocation / safe-Rust GEMM
microkernel work or `str_outer_join` sequential position/output materialization,
with fresh RCH same-worker baselines before any source edit.
