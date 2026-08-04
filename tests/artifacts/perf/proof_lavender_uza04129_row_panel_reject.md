# br-frankenpandas-uza04.129 df_dot row-panel reuse rejection

## Target

- Hotspot: `perf_profile df_dot 100000 6`.
- Baseline binary: `/data/projects/.scratch/cargo-target-lavender-fp-current/release-perf/examples/perf_profile`.
- Candidate binary: `/data/projects/.scratch/cargo-target-lavender-uza04129-after/release-perf/examples/perf_profile`.
- Candidate lever: reuse each packed right-side `B` panel across four 4-row `A` tiles before advancing the output column block.

## Baseline Evidence

- RCH build: `tests/artifacts/perf/lavender_uza04129_base_build_perf_profile.txt`, remote `vmi1227854`, `release-perf`, `cargo build -p fp-conformance --profile release-perf --example perf_profile`.
- Internal run: `perf_profile: done 6 iters in 0.703s (117.161 ms/iter), sink=600000`.
- Standalone hyperfine: `794.9 ms +/- 15.8 ms`.

## Isomorphism Proof

The candidate did not change the semantic contract:

- Output row and column ordering: unchanged. Each output value was written to the same `band[j * bw + local_row]` slot.
- Floating point order: unchanged per output cell. Each `C[i][j]` still accumulated `l = 0..k` in ascending order with the same `av * bp[dj]` then `+=` operation.
- NaN/null behavior: unchanged. Input coercion, `column_has_nan`, and fallback materialization paths were untouched.
- Tie-breaking: not applicable for dot output; row/column order remained the deterministic tie-break.
- RNG: not used.

Golden outputs matched byte-for-byte:

```text
ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535  df_dot_2000
04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d  df_dot_5000
```

Proof files:

- `tests/artifacts/perf/lavender_uza04129_base_golden.sha256`
- `tests/artifacts/perf/lavender_uza04129_after_golden.sha256`
- `tests/artifacts/perf/lavender_uza04129_base_golden_check.txt`
- `tests/artifacts/perf/lavender_uza04129_after_golden_check.txt`

## Benchmark Gate

The first paired forward/reversed hyperfine jobs were run concurrently and are contaminated by mutual CPU contention. They are retained only as diagnostic artifacts:

- `tests/artifacts/perf/lavender_uza04129_pair_forward.txt`
- `tests/artifacts/perf/lavender_uza04129_pair_reversed.txt`

Clean serial forward:

```text
baseline:  766.8 ms +/- 10.1 ms
candidate:   1.476 s +/- 0.064 s
baseline ran 1.92 +/- 0.09 times faster
```

Clean serial reversed:

```text
candidate:   1.501 s +/- 0.070 s
baseline:  801.4 ms +/- 34.5 ms
baseline ran 1.87 +/- 0.12 times faster
```

Decision: reject. Impact is negative, confidence is high across forward and reversed serial runs, and the source hunk was removed.

## Profiling Notes

`perf stat/record` was blocked by host policy:

```text
perf_event_paranoid setting is 4
```

The timing evidence is still sufficient to reject this lever. Next route must avoid the row-panel, BI/BJ widening, statement-unroll, worker-cap, and finite-input-scan families. Reprofile before selecting the next `[perf]` bead or fallback hotspot.
