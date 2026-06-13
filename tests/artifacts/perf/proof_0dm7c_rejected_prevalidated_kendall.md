# br-frankenpandas-0dm7c Supplemental Rejection Proof

## Candidate

Single lever tested:
hoist Kendall order/rank bounds validation out of the hot
`count_ordered_rank_inversions` loop for the complete no-tie matrix path. The
candidate kept the public checked helper unchanged, added a one-time matrix
input validation pass, and used a prevalidated inversion helper inside
`complete_kendall_no_tie_parallel_matrix`.

The source hunk was removed after the benchmark gate failed.

## Build Evidence

Matched comparator head: `9ebc32e7`.

Baseline worktree:
`/data/projects/.scratch/frankenpandas-orangepeak-0dm7c-base2`

After worktree:
`/data/projects/.scratch/frankenpandas-orangepeak-0dm7c-after`

Both release-perf example builds ran remotely on `vmi1227854`:

```bash
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-0dm7c-base2 \
RUSTFLAGS='-C force-frame-pointers=yes' \
rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile

CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-0dm7c-after \
RUSTFLAGS='-C force-frame-pointers=yes' \
rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Focused test:

```bash
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-0dm7c-test \
rch exec -- cargo test -p fp-frame kendall --lib
```

RCH fell back local for that test command; all six focused Kendall tests passed.

## Golden Proof

```text
031978ba431260b942dd36d9be055064a7453118a7c00888c35747644d33d99e  tests/artifacts/perf/0dm7c_base2_golden_df_kendall_5000.txt
031978ba431260b942dd36d9be055064a7453118a7c00888c35747644d33d99e  tests/artifacts/perf/0dm7c_after_golden_df_kendall_5000.txt
cmp_5000=0
```

Isomorphism obligations:

- Output order: unchanged; the matrix path still fills the same upper triangle
  and mirrors it to the transpose.
- Diagonal: unchanged at exact `1.0`.
- Symmetry: unchanged; each pair value is copied to its transpose.
- Tie/NaN fallback: unchanged; columns that cannot produce complete finite
  no-tie orders still use the existing fallback path.
- Floating point: unchanged for valid inputs; the same exact discordant count is
  converted with the same formula.
- RNG: unchanged; the workload has no RNG.

## Timings

Forward paired gate:

| Workload | Baseline | After | Verdict |
| --- | ---: | ---: | --- |
| `df_kendall 50000 3` | `439.8 ms +/- 16.3` | `435.0 ms +/- 7.2` | after `1.01x +/- 0.04`, noise |
| `df_kendall 200000 1` | `691.2 ms +/- 25.9` | `707.3 ms +/- 15.4` | baseline `1.02x +/- 0.04` faster |

Reversed paired gate:

| Workload | After | Baseline | Verdict |
| --- | ---: | ---: | --- |
| `df_kendall 200000 1` | `704.0 ms +/- 26.5` | `683.4 ms +/- 20.1` | baseline `1.03x +/- 0.05` faster |

The lever is flat on the smaller shape and regresses on the larger shape.

## Profile Limitation

Kernel sampling is blocked on this host:

```text
perf_event_paranoid setting is 4
perf_status=255
```

The failed profile artifact is retained at
`tests/artifacts/perf/0dm7c_base2_perf_record_df_kendall_200000x1.txt`.

## Decision

Rejected:

```text
Impact 0 x Confidence 4 / Effort 1 = 0.0
```

This confirms the Kendall path should not spend another pass on Fenwick
validation/cache-layout micro-levers. The next live route is
`br-frankenpandas-uza04.88`: exact all-pairs Kendall cross-pair work sharing or
another batched rank primitive that changes the amount of work while preserving
the same inversion counts and output bits.
