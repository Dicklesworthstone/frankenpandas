# br-frankenpandas-m0gcq closeout proof

## Target

- Bead: `br-frankenpandas-m0gcq`
- Workload: `fp-bench --category groupby --workload groupby_agg_multi --size 1M --dtype float64 --json`
- Profile-backed reason: the bead recorded `groupby_agg_multi` as the only
  remaining 1M-scale float64 operation slower than pandas (`fp=31053us`,
  `pd=22829us`, ratio `1.36x`).
- Runtime implementation: co-landed before this proof closeout in
  `07112d63`, `e04cd540`, and `e37bbd6b`. This artifact adds an independent
  deterministic `perf_profile` golden scenario plus paired hyperfine evidence
  from the baseline worktree and current candidate.

## Lever

The retained implementation routes typed `agg_list` / `agg_dict_list` /
`agg_multi` requests through typed grouped reducers and the dense Float64
moment path for compatible `sum`, `mean`, `count`, `var`, and `std` specs. The
hot path builds group ids once, accumulates group `sum` and `count` in source
row order, performs the existing two-pass centered sum-of-squares for
`var/std`, and emits output columns in the requested spec order.

This closeout commit does not add a second performance lever. It records the
reproducible proof surface and fixes the local lint/format debt required for
the focused validation gate to run cleanly.

## Baseline

- Baseline source: detached clean worktree from `29a96cf982` with the same
  `perf_profile groupby_agg_multi` scenario patched in for apples-to-apples
  golden capture.
- Baseline internal artifact:
  `tests/artifacts/perf/lavender_m0gcq_base_fp_bench_groupby_agg_multi_1m.json`
- Baseline standalone hyperfine:
  `tests/artifacts/perf/lavender_m0gcq_base_hyperfine_fp_bench_groupby_agg_multi_1m.txt`
  - mean: `1.090 s +/- 0.019 s`

## Re-benchmark

Forward paired hyperfine:

- Artifact:
  `tests/artifacts/perf/lavender_m0gcq_pair_hyperfine_fp_bench_groupby_agg_multi_1m.txt`
- Baseline: `1.096 s +/- 0.047 s`
- Candidate: `477.5 ms +/- 16.1 ms`
- Delta: candidate `2.29 +/- 0.13x` faster

Reversed paired hyperfine:

- Artifact:
  `tests/artifacts/perf/lavender_m0gcq_pair_rev_hyperfine_fp_bench_groupby_agg_multi_1m.txt`
- Candidate: `494.3 ms +/- 19.8 ms`
- Baseline: `1.086 s +/- 0.029 s`
- Delta: candidate `2.20 +/- 0.11x` faster

Score: Impact `4` * Confidence `5` / Effort `2` = `10.0`; accepted.

## Golden output

Scenario:

```text
perf_profile golden groupby_agg_multi 5000
```

Baseline SHA256:

```text
4b7c746e9413cb9f16da963e2ba248db8d538bd5a8b56631ac040aeb368d3b8a  tests/artifacts/perf/lavender_m0gcq_base_golden_groupby_agg_multi_5000.txt
```

Candidate SHA256:

```text
4b7c746e9413cb9f16da963e2ba248db8d538bd5a8b56631ac040aeb368d3b8a  tests/artifacts/perf/lavender_m0gcq_candidate_golden_groupby_agg_multi_5000.txt
```

Comparison artifact:
`tests/artifacts/perf/lavender_m0gcq_golden_compare.txt` contains `MATCH`.

## Isomorphism proof

- Ordering preserved: yes. `group_order` still comes from `build_groups()`;
  `labels` are still derived from the first row of each group; output columns
  are emitted in the requested `(column, func)` spec order.
- Tie-breaking unchanged: yes. Group key order and column order are inherited
  from the same source frame and spec sequence; no new sort is introduced.
- Floating-point behavior: preserved for the accepted workload. The dense path
  accumulates `sum/count` in source row order and computes `var/std` through
  the same two-pass centered formula used by the typed grouped reducers.
  The baseline/candidate golden SHA above is byte-identical.
- Null/NaN behavior: preserved for this all-valid Float64 benchmark lane. The
  typed path is gated by `as_f64_slice()`; incompatible/null-bearing cases stay
  on existing fallback routes.
- RNG: N/A. The workload and implementation are deterministic and use no RNG.

## Validation

- `rustfmt --check crates/fp-frame/src/lib.rs crates/fp-conformance/examples/perf_profile.rs`: pass after scoped formatting.
- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavender-m0gcq-validate2 rch exec -- cargo check -p fp-conformance --example perf_profile`: pass remotely on `vmi1227854`.
- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavender-m0gcq-validate2 rch exec -- cargo clippy -p fp-conformance --example perf_profile -- -D warnings`: pass via `rch` local fallback (`no admissible workers`).
- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavender-m0gcq-validate2 rch exec -- cargo test -p fp-frame dataframe_groupby_agg_multi_per_column_multiple_funcs --lib -- --nocapture`: pass via `rch` local fallback (`no admissible workers`).
- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavender-m0gcq-validate2 rch exec -- cargo test -p fp-conformance packet_filter_runs_dataframe_groupby_agg_multi_packet -- --nocapture`: pass via `rch` local fallback (`no admissible workers`).
- `git diff --check -- crates/fp-frame/src/lib.rs crates/fp-conformance/examples/perf_profile.rs tests/artifacts/perf/proof_lavender_m0gcq_fused_groupby_multiagg_closeout.md`: pass.
- `ubs crates/fp-frame/src/lib.rs crates/fp-conformance/examples/perf_profile.rs`: timed out at 180s with `UBS_EXIT=124`; artifact:
  `tests/artifacts/perf/lavender_m0gcq_ubs_touched_rust.txt`.
