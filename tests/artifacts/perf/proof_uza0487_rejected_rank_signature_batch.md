# br-frankenpandas-uza04.87 - rejected row-major rank-signature batch

## Candidate

Rejected source candidate: transpose Kendall `rank_by_row` witnesses into a row-major rank-signature slab and, for each left-column order, process target columns in batches of four with separate word-block counters.

The intended win was cross-pair work sharing: one traversal of `x_order[i]` would feed a small target-column batch, with contiguous rank-signature loads for the target ranks.

The source hunk was removed after measurement. No runtime source change is kept for this bead.

## Baseline

Current post-`br-frankenpandas-uza04.90` baseline was rebuilt with:

```bash
RUSTFLAGS='-C force-frame-pointers=yes' CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0487-base rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Build artifact: `tests/artifacts/perf/uza0487_base_build_perf_profile.txt`.

Fresh baseline-only timings:

- `df_kendall 50000 1`: `120.5 ms +/- 5.5 ms`
- `df_kendall 200000 1`: `495.7 ms +/- 9.0 ms`

`perf stat` remained blocked by `perf_event_paranoid=4`; `/usr/bin/time -v` for `200000x1` reported wall `0.49s`, CPU `736%`, max RSS `310268 KB`.

## Golden proof

Baseline and candidate output bytes matched for all required goldens:

| Scenario | SHA256 |
| --- | --- |
| `df_kendall 2000` | `acf366c266b66f8497fb55734ed1b4ec40952a7c014f43db262c7c1e625e15e1` |
| `df_kendall 5000` | `031978ba431260b942dd36d9be055064a7453118a7c00888c35747644d33d99e` |
| `df_kendall 20000` | `f164fa86300fbea93e46aa99a5e0f7413fa8c0baf2ee49c6a689ed92652a4c3b` |

Artifacts:

- `tests/artifacts/perf/uza0487_base_golden_sha256.txt`
- `tests/artifacts/perf/uza0487_after_golden_sha256.txt`
- `tests/artifacts/perf/uza0487_after_golden_check.txt`
- `tests/artifacts/perf/uza0487_after_golden_cmp.txt`

All `cmp` statuses were `0`.

## Benchmarks

Paired forward order:

| Scenario | Baseline mean | Candidate mean | Verdict |
| --- | ---: | ---: | --- |
| `df_kendall 50000 1` | `117.203 ms` | `146.481 ms` | baseline `1.25x` faster |
| `df_kendall 200000 1` | `531.250 ms` | `651.087 ms` | baseline `1.23x` faster |

Paired reversed order:

| Scenario | Baseline mean | Candidate mean | Verdict |
| --- | ---: | ---: | --- |
| `df_kendall 50000 1` | `115.121 ms` | `151.937 ms` | baseline `1.32x` faster |
| `df_kendall 200000 1` | `495.715 ms` | `711.344 ms` | baseline `1.43x` faster |

Benchmark artifacts:

- `tests/artifacts/perf/uza0487_pair_forward_hyperfine_df_kendall_50000x1.json`
- `tests/artifacts/perf/uza0487_pair_forward_hyperfine_df_kendall_200000x1.json`
- `tests/artifacts/perf/uza0487_pair_reversed_hyperfine_df_kendall_50000x1.json`
- `tests/artifacts/perf/uza0487_pair_reversed_hyperfine_df_kendall_200000x1.json`

## Validation

Focused Kendall tests passed while the candidate source was present:

- `tests/artifacts/perf/uza0487_candidate_focused_kendall_tests.txt`

## Isomorphism

- Ordering preserved: yes, candidate used the same `x_order` and `rank_by_row` witnesses.
- Tie-breaking unchanged: yes, candidate remained gated by complete no-tie ordered ranks.
- Floating-point unchanged: yes, candidate only changed integer discordance-count scheduling; golden bytes matched.
- RNG unchanged: N/A.
- NaN/null fallback unchanged: yes, admission and fallback remained outside the candidate helper.

## Verdict

Rejected. Score is below `2.0` because impact is negative. The row-major rank-signature batch reduced some repeated `x_order` traversal but added enough per-batch counter state and rank-signature traffic to lose decisively.

Do not retry small-lane row-major target batching for Kendall. The next deeper primitive should be a different algorithmic family, such as static wavelet/rank-select witnesses or divide-and-conquer dominance summaries that reduce per-pair rank-query work rather than batching the same dynamic counters.
