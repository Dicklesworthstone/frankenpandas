# Lane M: stateful `Expanding.apply` versus pandas at 1M and 10M

## Result

**Campaign result class:** `incumbent-win`.

The canonical gate compares FrankenPandas `Expanding::apply` with the fastest
task-equivalent pandas 2.2.3 route found in an eight-route same-worker 1M
screen. Both engines consume the exact sequence `value[i] = i % 997` and
execute the ordered recurrence

```text
prefix_len = i + 1
state = (state * 31 + value[i] + prefix_len) & 0x7fffffff
output[i] = state
```

Each result depends on the newest member and length of the growing prefix plus
every preceding callback invocation. Input construction remains outside the
timed region on both engines.

| size | FP p50 | pandas p50 | pandas / FP | effect / required | verdict |
|---:|---:|---:|---:|---:|---|
| 1M | 22.774 ms | 563.862 ms | **24.759x** | 3.20917814 / 0.62273492 | **FASTER** |
| 10M | 299.330 ms | 5,982.580 ms | **19.987x** | 2.99506041 / 0.22873088 | **FASTER** |

The two-row geomean is **22.245x**. Absolute median time saved grows from
541.087 ms to 5.683 seconds, while the ratio contracts 19.3%. This is a
decisive ordered expanding-callback incumbent win, but it is not a
ratio-amplifying Class-1 result on this ELF.

## Strongest-incumbent screen

An eight-route pandas 2.2.3 screen ran at 1M on the canonical worker
`vmi1264463`. Every route produced the exact same Float64 Series and final
state: length 1,000,000, first value `1.0`, final value and state
`1311869426`.

| pandas route | p50 |
|---|---:|
| stateful scalar callback + `np.fromiter(map(...))` | **491.419 ms** |
| stateful generator + `np.fromiter` | 511.099 ms |
| `itertools.accumulate` + `np.fromiter` | 526.040 ms |
| `Series.apply` | 586.065 ms |
| `Series.transform` | 606.179 ms |
| `Series.map` | 628.080 ms |
| exact `Expanding.apply(raw=True)` | 1,071.759 ms |
| exact `Expanding.apply(raw=False)` | 29,797.105 ms |

The 491.419 ms route advanced to the canonical 1M/10M gate. The exact pandas
API's larger idiom penalty is route-screen evidence only and is not used in
the competitive headline.

The first gate used the locally fastest generator route. The same-worker
screen then showed `np.fromiter(map(...))` was 3.9% faster, so no campaign
claim is based on that first gate. Its raw JSON is retained as a
weaker-incumbent diagnostic; the map-arm invocation below is the sole
canonical result.

## Admission contract

**Legacy incumbent arm (same invocation):**

```text
name=pandas version=2.2.3
artifact_sha256=051be80fe43b4e0be4e04af314c42db966950eb877b0634d482099f42535e9bb
invocation_id=vs-pandas-20260729T034520.201126Z-pid3599005
measured_ratio=19.987x
```

**Executing ELF SHA-256 (self-reported by process):**

```text
bench_elf_sha256=f3e65361ae1f089b2b3d2b95f6938a046b558c1aa14ab04a4ee15c4bfaffd86e
(70379320 bytes)
/data/projects/frankenpandas/.rch-target-vmi1264463-pool-bc445989bdf88102bcbc62abd4347d69/release-perf/fp-bench
```

The worker's direct post-run SHA-256 matched the in-process self-report. The
canonical harness source SHA-256 was
`126797478b435c4021ae1d8b71f1094b1813327764c786acf4af424037cafcbe`.

**A/A null control (same invocation):** 25 alternating pairs per engine and
row. FP/pandas bootstrap-median 95% CIs were
`[0.973233,1.028115]` / `[0.732445,1.055421]` at 1M and
`[0.918880,1.114410]` / `[0.990895,1.121162]` at 10M.

**Median-CI decision:** median effect 3.20917814 cleared the required
log-effect threshold 0.62273492 at 1M; median effect 2.99506041 cleared the
required log-effect threshold 0.22873088 at 10M.

**CV role:** provenance only; CV had no vote. FP/pandas CV was
43.80%/35.84% at 1M and 43.77%/22.89% at 10M.

## Semantic and build evidence

- Every pandas route in the same-worker screen produced the same values,
  Float64 dtype, index, length, and final callback state.
- Rust and pandas start from state zero and observe the same exact integer
  value and one-based prefix length at each row. The masked state is below
  `2^31`; multiplication and additions remain below `2^36`, so Rust wrapping
  arithmetic and Python integer arithmetic feed the same recurrence. Induction
  therefore gives identical output at every row.
- The Rust recurrence fixture locks the first three outputs
  `[1.0, 34.0, 1059.0]` and final state.
- The strict-remote focused recurrence test passed 1/1 on `vmi1264463`.
- The canonical raw artifact passed the schema-v4
  ELF/A/A/median-CI contract check.

Strict-remote command:

```text
RCH_WORKER=vmi1264463 RCH_REQUIRE_REMOTE=1 env -u CARGO_TARGET_DIR \
  rch exec -- cargo run --locked --profile release-perf -p fp-bench -- \
  --remote-python-harness --category rolling --sizes 1M,10M \
  --dtypes float64 --workloads expanding_apply_stateful \
  --output artifacts/bench/proud_lane_m_expanding_apply_stateful_map_canonical_1m_10m_20260728.json \
  --json-stdout
```

## Decision and retry predicate

**Decision: KEEP** the incumbent harness coverage and both admitted
`incumbent-win` rows. This is measurement-only campaign output, not an
FP-before/FP-after self-speedup or a production source lever.

Keep the ratios until the input sequence, recurrence, expanding
`min_periods`, FrankenPandas expanding callback implementation, harness
source, pandas or NumPy artifact, allocator, compiler, worker ISA, or
executing ELF changes. Re-open a ratio-growth claim only when one
self-identified ELF runs both sizes on one worker in one invocation and two
independent gates put the 10M ratio above the 1M ratio outside the combined
A/A intervals. Re-screen all callable pandas routes after a pandas or NumPy
version change or worker-class change. Retry 10M for a tail-latency claim only
with fresh child processes plus peak RSS and major-fault counters. Do not infer
a production `Expanding::apply` lever without a current profile naming a
non-zero-self frame and a computed Amdahl ceiling.

## Raw artifacts

Canonical map-arm gate:
`proud_lane_m_expanding_apply_stateful_map_canonical_1m_10m_20260728.json`,
SHA-256
`eecebb7761ed1a6bfd87ac33243a991c6f073c3c63fccbff060f74f78dc13cb2`.

Generator-arm diagnostic, not used for the competitive claim:
`proud_lane_m_expanding_apply_stateful_1m_10m_20260728.json`, SHA-256
`f5181b63d20158755ef891d45ed01020c3ba6eaac131b8218901d5bff790b156`.
