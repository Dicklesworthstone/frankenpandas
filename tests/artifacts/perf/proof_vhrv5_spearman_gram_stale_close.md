# br-frankenpandas-vhrv5 - Spearman Gram stale closeout

Status: closed as already satisfied in current `main`. No runtime source changed.

## Bead Target

`br-frankenpandas-vhrv5` asked to route complete-column Spearman correlation
through the existing Gram kernel instead of the old per-pair centered-rank dot
loop.

Current code already has that route:

- `complete_spearman_gram_matrix`
- `pairwise_rank_corr("spearman")` calls `complete_spearman_gram_matrix(...)`
  before falling back to `complete_spearman_parallel_matrix(...)`.

## Fresh Baseline

Baseline binary:

`/data/projects/.scratch/cargo-target-lavenderstone-vhrv5-base/release-perf/examples/perf_profile`

RCH build:

`CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavenderstone-vhrv5-base RUSTFLAGS="-C force-frame-pointers=yes" rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`

RCH worker: `vmi1153651`.

Golden-output SHA256:

- `df_spearman 5000`: `dc37f75e1ee2e23e28ed89e0f178a0268cb65cc420bc82f97cb16d219eb7dd1e`

Baseline hyperfine:

- `df_spearman 50000x1`: `93.2 ms +/- 6.1 ms`
- `df_spearman 50000x5`: `329.6 ms +/- 9.0 ms`

Artifacts:

- `tests/artifacts/perf/vhrv5_baseline_df_spearman_50000.json`
- `tests/artifacts/perf/vhrv5_baseline_df_spearman_50000_5iters.json`

## Decision

Close as stale/already satisfied.

The bead notes cite an older `df_spearman` runtime around `210 ms` and mention
the read-once Gram route as already landed. Current `main` has the Gram route
and the fresh RCH-built baseline is `93.2 ms +/- 6.1 ms` for `50000x1`.

No source edit is needed for this bead. Continuing to tune this same stale
Spearman surface would repeat the already-landed Gram route or the rejected
radix-rank family.

## Next Route

Move to the next ready profile-backed perf bead instead of repeating the stale
Spearman route. The next live target should be one of the affine filter residual
beads (`br-frankenpandas-uza04.51` / `.76`) after verifying which one is not
stale and has a current profiler-evident hotspot.
