# Lane M: stateful `Rolling.apply` versus pandas at 1M and 10M

## Result

**Campaign result class:** `incumbent-win`.

The canonical gate compares FrankenPandas `Rolling::apply` with the fastest
task-equivalent pandas 2.2.3 route found in an eight-route 1M screen. Both
engines compute a width-10 rolling sum over the exact sequence
`value[i] = i % 997`, then execute the ordered recurrence

```text
state = (state * 31 + window_sum) & 0x7fff_ffff
output[i] = state
```

The first nine outputs are missing. Every later output depends on every
preceding callback invocation, so neither a built-in rolling aggregation nor
an unordered vectorized operation can replace the callback while preserving
the observed Series.

| size | FP p50 | pandas p50 | pandas / FP | effect / required | verdict |
|---:|---:|---:|---:|---:|---|
| 1M | 45.768 ms | 452.062 ms | **9.877x** | 2.29023087 / 0.16091064 | **FASTER** |
| 10M | 667.503 ms | 4,449.835 ms | **6.666x** | 1.89707813 / 0.64651052 | **FASTER** |

The two-row geomean is **8.114x**. Absolute median time saved grows from
406.294 ms to 3.782 seconds, while the ratio contracts 32.5%. This is a
decisive ordered rolling-callback incumbent win, but it is not a
ratio-amplifying Class-1 result on this ELF.

## Strongest-incumbent screen

An eight-route pandas 2.2.3 screen ran at 1M after input construction. Every
route produced the exact same Float64 Series and final state: length
1,000,000, nine leading missing values, first valid value `45.0`, final value
and state `1700120680`.

| pandas route | p50 |
|---|---:|
| `rolling.sum()` + stateful generator + `np.fromiter` | **174.846 ms** |
| `rolling.sum()` + `itertools.accumulate` | 186.628 ms |
| `rolling.sum()` + `ufunc.accumulate` | 204.617 ms |
| `rolling.sum()` + `Series.map` | 248.072 ms |
| `rolling.sum()` + `Series.apply` | 282.538 ms |
| `rolling.sum()` + `Series.transform` | 299.585 ms |
| exact `rolling.apply(raw=True)` | 1,417.559 ms |
| exact `rolling.apply(raw=False)` | 22,679.292 ms |

The 174.846 ms route therefore advanced to the canonical 1M/10M gate. The
exact pandas API is retained only as route-screen evidence; its much larger
idiom penalty is not used in the competitive headline. Numba was not
installed on the benchmark environment, and the stateful cross-window
semantics cannot be represented by pandas' independent-window numba callback
contract.

## Admission contract

**Legacy incumbent arm (same invocation):**

```text
name=pandas version=2.2.3
artifact_sha256=051be80fe43b4e0be4e04af314c42db966950eb877b0634d482099f42535e9bb
invocation_id=vs-pandas-20260729T025159.104971Z-pid3510691
measured_ratio=6.666x
```

**Executing ELF SHA-256 (self-reported by process):**

```text
bench_elf_sha256=30ad2f887b84c55faf7ef921c1d1b21f17b8ec941927b80a17ea6eadd500f11d
(70357352 bytes)
/data/projects/frankenpandas/.rch-target-vmi1264463-pool-bc445989bdf88102bcbc62abd4347d69/release-perf/fp-bench
```

The worker's direct post-run SHA-256 matched the in-process self-report. The
harness source SHA-256 was
`6330f14ab9fcd4d189d5609bbc4d40c007fd50bee3fc03c5c7a38ff97929b51f`.

**A/A null control (same invocation):** 25 alternating pairs per engine and
row. FP/pandas bootstrap-median 95% CIs were
`[0.995149,1.079558]` / `[0.946752,1.083780]` at 1M and
`[0.723789,1.364822]` / `[0.980225,1.027574]` at 10M.

**Median-CI decision:** median effect 2.29023087 cleared the required
log-effect threshold 0.16091064 at 1M; median effect 1.89707813 cleared the
required log-effect threshold 0.64651052 at 10M.

**CV role:** provenance only; CV had no vote. FP/pandas CV was
11.38%/21.62% at 1M and 78.22%/7.56% at 10M.

## Semantic and build evidence

- Every pandas route in the screen produced the same values, dtype, index,
  missing prefix, length, and final callback state.
- Integer-valued width-10 windows have sums below 9,970, exactly representable
  in Float64. The Rust callback and pandas route therefore feed identical
  integer window sums into the same recurrence.
- The Rust recurrence fixture locks the first three valid outputs
  `[3.0, 99.0, 3078.0]` and final state for width-three test windows.
- The strict-remote focused recurrence test passed 1/1 on `vmi1264463`.
- The canonical raw artifact passed the schema-v4
  ELF/A/A/median-CI contract check.

Strict-remote command:

```text
RCH_WORKER=vmi1264463 RCH_REQUIRE_REMOTE=1 env -u CARGO_TARGET_DIR \
  rch exec -- cargo run --locked --profile release-perf -p fp-bench -- \
  --remote-python-harness --category rolling --sizes 1M,10M \
  --dtypes float64 --workloads rolling_apply_stateful \
  --output artifacts/bench/proud_lane_m_rolling_apply_stateful_1m_10m_20260728.json \
  --json-stdout
```

## Decision and retry predicate

**Decision: KEEP** the incumbent harness coverage and both admitted
`incumbent-win` rows. This is measurement-only campaign output, not an
FP-before/FP-after self-speedup or a production source lever.

Keep the ratios until the rolling width, input sequence, recurrence,
FrankenPandas rolling callback implementation, harness source, pandas or
NumPy artifact, allocator, compiler, worker ISA, or executing ELF changes.
Re-open a ratio-growth claim only when one self-identified ELF runs both sizes
on one worker in one invocation and two independent gates put the 10M ratio
above the 1M ratio outside the combined A/A intervals. Re-screen all callable
pandas routes after a pandas or NumPy version change. Retry 10M for a
tail-latency claim only with fresh child processes plus peak RSS and
major-fault counters. Do not infer a production `Rolling::apply` lever without
a current profile naming a non-zero-self frame and a computed Amdahl ceiling.

## Raw artifact

`proud_lane_m_rolling_apply_stateful_1m_10m_20260728.json`, SHA-256
`811a0c96f76d12e31ef4f72feaaf0a2e0c57401264fa9a6e80ca52a324de305b`.
