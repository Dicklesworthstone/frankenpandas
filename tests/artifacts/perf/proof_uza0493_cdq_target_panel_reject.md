# br-frankenpandas-uza04.93 rejection proof

## Change

Tested a fixed-left CDQ target-panel Kendall row builder for
`complete_kendall_no_tie_parallel_matrix`.

The candidate inverted one left column order into `left_pos_by_row`, processed
all right columns for that left through a shared divide-and-conquer partition,
and counted each unordered row pair at its split ancestor. Runtime source was
removed after the perf gate failed.

## Baseline

- Baseline binary:
  `/data/projects/.scratch/cargo-target-lavenderstone-uza0493-base/release-perf/examples/perf_profile`
- Baseline build command:
  `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavenderstone-uza0493-base RUSTFLAGS="-C force-frame-pointers=yes" rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- RCH result: failed open locally because there were no admissible workers:
  `insufficient_slots=2,hard_preflight=10`.
- Baseline-only timing:
  - `df_kendall 50000x1`: `120.3 ms +/- 6.2 ms`
  - `df_kendall 200000x1`: `510.0 ms +/- 15.8 ms`

`perf stat` was blocked by host policy (`perf_event_paranoid=4`), so the target
was backed by the existing profile evidence for the Kendall inversion path plus
fresh hyperfine baselines.

## Proof

Focused parity tests passed on RCH worker `vmi1227854`:

- `cargo test -p fp-frame --lib kendall_cdq_target_panel_matches_serial_adversarial_permutations -- --nocapture`
- `cargo test -p fp-frame --lib complete_kendall_parallel_matrix_matches_serial_ordered_ranks -- --nocapture`

Golden output SHAs were unchanged before and after:

- `df_kendall 2000`: `acf366c266b66f8497fb55734ed1b4ec40952a7c014f43db262c7c1e625e15e1`
- `df_kendall 5000`: `031978ba431260b942dd36d9be055064a7453118a7c00888c35747644d33d99e`
- `df_kendall 20000`: `f164fa86300fbea93e46aa99a5e0f7413fa8c0baf2ee49c6a689ed92652a4c3b`

Isomorphism:

- Ordering preserved: output matrix diagonal, upper-triangle order, and symmetry
  copy were unchanged.
- Tie-breaking unchanged: the complete-finite/no-tie gate remained before the
  candidate path; fallback semantics were untouched.
- Floating point unchanged: the candidate produced the same integer discordance
  counts and used the existing final `f64` formula.
- RNG unchanged: no RNG use.

## Benchmark

After binary was built on RCH worker `vmi1153651`:

`CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavenderstone-uza0493-after RUSTFLAGS="-C force-frame-pointers=yes" rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`

Paired same-host hyperfine gates:

| Gate | Baseline | Candidate | Verdict |
| --- | ---: | ---: | --- |
| `50000x1` forward | `120.0 ms +/- 3.6 ms` | `315.8 ms +/- 12.9 ms` | baseline `2.63x +/- 0.13` faster |
| `50000x1` reversed | `119.4 ms +/- 5.8 ms` | `309.3 ms +/- 4.1 ms` | baseline `2.59x +/- 0.13` faster |
| `200000x1` forward | `498.0 ms +/- 12.8 ms` | `1.428 s +/- 0.021 s` | baseline `2.87x +/- 0.08` faster |
| `200000x1` reversed | `511.4 ms +/- 14.9 ms` | `1.446 s +/- 0.030 s` | baseline `2.83x +/- 0.10` faster |

Artifacts:

- `tests/artifacts/perf/uza0493_baseline_df_kendall_50000.json`
- `tests/artifacts/perf/uza0493_baseline_df_kendall_200000.json`
- `tests/artifacts/perf/uza0493_pair_df_kendall_50000_forward.json`
- `tests/artifacts/perf/uza0493_pair_df_kendall_50000_reversed.json`
- `tests/artifacts/perf/uza0493_pair_df_kendall_200000_forward.json`
- `tests/artifacts/perf/uza0493_pair_df_kendall_200000_reversed.json`

## Decision

Reject. Score is below the keep gate because the candidate regressed every
timing gate. Runtime source hunk and focused candidate test were removed.

Root cause: the CDQ target-panel shares recursion shape, but it still streams
and repartitions every target column at every recursion level. For the current
32-column matrix shape that increases memory traffic and copy work versus the
existing word-blocked rank counter.

Next route: attack a different primitive, not CDQ target-panel repartitioning.
The next candidate should use a cache-oblivious / succinct bitplane witness that
computes exact Kendall discordance from row-order rank bitplanes with fewer
per-level copies, while preserving the current golden outputs.
