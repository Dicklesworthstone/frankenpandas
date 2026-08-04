# br-frankenpandas-uza04.50 proof: rejected nullable validity range-clear lever

## Target

- Scenario: `perf_profile outer_join 500000 20`
- Baseline binary: `/data/projects/.scratch/cargo-target-orangepeak-uza0450-base/release-perf/examples/perf_profile`
- Candidate binary: `/data/projects/.scratch/cargo-target-orangepeak-uza0450-after/release-perf/examples/perf_profile`
- Profile-backed target: nullable outer-join output construction showed samples under `Column::from_f64_nullable_repeated_slices_shared`, `Column::from_f64_nullable_repeat_values_run_lengths`, and `__memset_avx2_unaligned_erms`.

## Candidate lever

Replace per-bit nullable-validity clearing with word-range clearing in `ValidityMask`, used by nullable repeated-slice and nullable repeat-value constructors.

The lever was removed after measurement. No production code from this candidate is retained.

## Isomorphism and golden proof

- Ordering: unchanged; no join planner, row ordering, hash/tie-breaking, or column-ordering path was retained.
- Floating point: unchanged; the candidate only changed validity-bit construction and was removed after rejection.
- RNG: not used by the scenario.
- Golden command: `perf_profile golden outer_join 20000`
- Baseline SHA256: `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`
- Candidate SHA256: `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`
- `cmp -s` baseline-vs-candidate golden output: pass

## Timing evidence

Baseline-only hyperfine:

- `1.242 s +/- 0.040`

Candidate direct timing:

- `74.142 ms/iter` internal timer

Paired hyperfine:

- Baseline: `1.221 s +/- 0.033`
- Candidate: `1.221 s +/- 0.038`
- Ratio: baseline ran `1.00x +/- 0.04` faster than candidate

## Decision

Rejected. Score is below the required keep threshold:

- Impact: 0
- Confidence: 5
- Effort: 1
- Score: `0 * 5 / 1 = 0`

Next route: stop iterating on nullable validity clearing. Re-profile and attack the deeper dense outer-merge materialization path or benchmark setup/frame construction only if the profile still supports it.
