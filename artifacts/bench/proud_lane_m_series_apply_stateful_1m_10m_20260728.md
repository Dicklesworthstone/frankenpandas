# Lane M: stateful Series callback versus pandas at 1M and 10M

## Result

**Campaign result class:** `incumbent-win`.

The canonical gate compares FrankenPandas `Series::apply` with pandas 2.2.3
`Series.map`, the fastest task-equivalent pandas route in a six-way 1M screen.
Both engines execute the same ordered recurrence over `0..n`:

```text
state = (state * 31 + value) & 0x7fff_ffff
output[i] = state
```

The output therefore depends on every preceding callback invocation. This is
not the rejected row-sum dispatch trap: replacing the callback with a built-in
reduction does not preserve the output sequence.

| size | FP p50 | pandas p50 | pandas / FP | effect / required | verdict |
|---:|---:|---:|---:|---:|---|
| 1M | 32.628 ms | 692.448 ms | **21.223x** | 3.05506638 / 0.10051494 | **FASTER** |
| 10M | 433.931 ms | 7,945.613 ms | **18.311x** | 2.90749025 / 0.44077829 | **FASTER** |

The two-row geomean is **19.713x**. Absolute median time saved grows from
659.820 ms to 7.512 seconds, while the ratio contracts 13.7%. This is strong
evidence for compiled ordered-callback execution, but it is not a
ratio-amplifying Class-1 result on this ELF.

## Strongest-incumbent screen

A pandas 2.2.3 route screen ran three 1M samples per arm after population,
with every route producing the exact same Int64 output and final state
`2028822816`:

| pandas route | p50 |
|---|---:|
| `Series.map` | **313.244 ms** |
| `Series.transform` | 318.525 ms |
| `Series.apply` | 326.596 ms |
| `Series(np.fromiter(...))` | 359.640 ms |
| list comprehension + `Series` | 411.551 ms |
| `Series(map(...))` | 435.176 ms |

`Series.map` therefore advanced to the canonical 1M/10M gate. A separate
exact-`Series.apply` diagnostic invocation measured 21.963x and 18.484x; those
larger ratios are retained in the raw artifact but are not the headline.

## Admission contract

**Legacy incumbent arm (same invocation):**

```text
name=pandas version=2.2.3
artifact_sha256=051be80fe43b4e0be4e04af314c42db966950eb877b0634d482099f42535e9bb
invocation_id=vs-pandas-20260729T020458.526396Z-pid3412769
measured_ratio=21.223x
```

**Executing ELF SHA-256 (self-reported by process):**

```text
bench_elf_sha256=a51f3952bce4d8d551d3f2dac1536414d0a524096481748071fd6a1cae1cfc06
(70315096 bytes)
/data/projects/frankenpandas/.rch-target-vmi1264463-pool-bc445989bdf88102bcbc62abd4347d69/release-perf/fp-bench
```

Harness source SHA-256:
`4661081137c47bdfa48baba2917f45cd33456b08ac7e5b889815fcffb962408a`.

**A/A null control (same invocation):** 25 alternating pairs per engine and
row. FP/pandas bootstrap-median 95% CIs were
[0.984570, 1.030109]/[0.980584, 1.051542] at 1M and
[0.938003, 1.246562]/[0.969347, 1.077802] at 10M.

**Median-CI decision:** median effect 3.05506638 cleared the required
log-effect threshold 0.10051494 at 1M; median effect 2.90749025 cleared the
required log-effect threshold 0.44077829 at 10M.

**CV role:** provenance only; CV had no vote. FP/pandas CV was 10.52%/6.80%
at 1M and 92.51%/15.21% at 10M.

## Semantic and build evidence

- The Python route screen proved all six pandas routes value-for-value equal
  for the 1M Int64 output, including index, first value, last value, dtype,
  length, and final callback state.
- The Rust fixture test locks the first eight recurrence outputs and final
  state; equality with the Python recurrence then follows by induction from
  identical initial state, input order, mask, and recurrence.
- Strict-remote focused test:
  `harness_contract_tests::stateful_apply_fixture_is_order_dependent_and_deterministic`
  passed 1/1 on `vmi1264463`.
- The canonical raw artifact passed the schema-v4 ELF/A/A/median-CI contract
  check, and its worker/local SHA-256 matched.

## Decision and retry predicate

**Decision: KEEP** the incumbent harness coverage and the two admitted
`incumbent-win` rows. This is measurement-only campaign output, not an
FP-before/FP-after self-speedup or a production source lever.

Keep the ratios until the callback recurrence, input order, Series callback
implementation, harness source, pandas artifact, allocator, compiler, worker
ISA, or executing ELF changes. Re-open a ratio-growth claim only when one
self-identified ELF runs both sizes on one worker in one invocation and two
independent gates put the 10M ratio above the 1M ratio outside the combined
A/A intervals. Re-screen callable pandas routes after a pandas version change.
Do not infer a production `Series::apply` lever without a current profile
naming a non-zero-self frame and a computed Amdahl ceiling.

## Raw artifacts

- Canonical fastest-incumbent gate:
  `proud_lane_m_series_apply_stateful_fastest_map_1m_10m_20260728.json`
  (SHA-256
  `0cf53b4d18a28f34f4bb15a3eadb4d8244ea7a014751f146bc55df49b2a7dda5`)
- Exact pandas `Series.apply` diagnostic:
  `proud_lane_m_series_apply_stateful_1m_10m_20260728.json`
  (SHA-256
  `9645721969dc4930b50bcce0e522bec87d1b04632ee17c1b30de171660c97387`)
