# Lane M `dt.month_name` strongest-incumbent gate

Date: 2026-07-28 America/New_York

Bead: `br-frankenpandas-dsapf`

Result: **KEEP** the competitive claim. FrankenPandas reached 1.580x pandas
at 1M and 1.684x at 10M. Both wins are independently decidable under the
median-CI gate.

**Campaign result class:** `incumbent-win`.

## Incumbent admission and semantic proof

Both engines receive the same all-valid `datetime64[ns]` sequence:

```text
base = 946684800000000000  # 2000-01-01T00:00:00
value[i] = base + i * 600000000000  # 600 seconds
```

Population remains outside timing. A same-worker pandas 2.2.3 / NumPy 2.4.3
route screen on the exact 1M-row fixture used two warmups and seven
interleaved samples per route:

| pandas route | median | output |
|---|---:|---|
| `Series.dt.month_name()` | 266.402 ms | equal |
| `Series.dt.strftime("%B")` | 6,800.564 ms | equal |
| `Series.dt.month` + NumPy name gather + `Series(...)` | 52.999 ms | equal |
| `Series.array.month` + NumPy name gather + `Series(...)` | **38.571 ms** | equal |

All four results were exactly equal, including object dtype, Series index,
and name. They contained all 12 English month names; the first and last
1M-row values were both `January`. Only the fastest route advanced to the
canonical gate:

```text
codes = series.array.month
pd.Series(month_names[codes - 1], index=series.index, name=series.name)
```

This is a live pandas arm, not a NumPy-only output: the public pandas
DatetimeArray extracts the month codes and the timed region constructs the
complete result Series. The month-name table and datetime population remain
outside timing on both engines.

The FrankenPandas arm executes `series.dt().month_name()`. The strict-remote
focused `fp-frame` `dt_month_name` test passed before measurement. Production
source did not change.

Behavior-preservation checklist:

- Values: identical English month names for the same nanosecond timestamps.
- Order and cardinality: one result per input row in original order.
- Result contract: object/string Series with the same index and name.
- Null behavior: outside this all-valid fixture; retained by the existing
  production implementation and focused test.
- RNG: none.

## Measurement contract

- Worker: `vmi1264463` (`38.242.209.154`).
- Disk guard: 361 GiB free before each Cargo invocation, above the 120 GiB
  floor.
- Strict-remote command:

  ```text
  RCH_WORKER=vmi1264463 RCH_REQUIRE_REMOTE=1 env -u CARGO_TARGET_DIR rch exec -- cargo run --locked --profile release-perf -p fp-bench -- --remote-python-harness --category datetime --sizes 1M,10M --dtypes datetime64 --workloads dt_month_name --output artifacts/bench/proud_lane_m_dt_month_name_1m_10m_20260728.json --json-stdout
  ```

- Shared invocation ID:
  `vs-pandas-20260729T061653.224380Z-pid3877027`.
- In-process FP ELF SHA-256:
  `a3a1cf82c4a7f5e0dfee9e5cdbfd59caacc96270fa009b72ae65599529bf5438`
  (70,378,976 bytes). A direct worker hash matched.
- Rust benchmark source SHA-256:
  `bf155d88b2fc76b163b021375adb9b5674ba47ed4792253d1512285a47902cc4`.
  The worker and local hashes matched.
- Python harness source SHA-256:
  `87ce0065fcf194501f93ab140201cf8f25fc7d62ffe5138c11291d46e221a388`
  (70,442 bytes). The worker, local, and in-process reports matched.
- Python executable SHA-256:
  `efb29ce53d36ebaeee80e3aa44fd6c7f9d71bbded5fe1665240b2ed8ecaeee0e`.
- pandas 2.2.3 content-tree SHA-256:
  `051be80fe43b4e0be4e04af314c42db966950eb877b0634d482099f42535e9bb`.
- Every engine and size ran 25 alternating A/A pairs in the same invocation
  as its A/B comparison.
- Decision gate: twice the larger FP/pandas bootstrap-median 95% CI log
  half-width. CV is provenance only and had no vote.
- Raw JSON SHA-256:
  `94751388626aa6a7b7a047fd08702fc204361647c2df6da03356c1c7104cd7bf`.

## Provenance fail-closed check

The first attempted invocation,
`vs-pandas-20260729T060438.792833Z-pid3848722`, self-reported the prior
harness SHA-256
`251ac27c48c2f484da72af4d0ab58e1e22c310b5b6c707879bbba5df267b4d75`
and rejected `dt_month_name` as unknown before any row ran. That invocation
produced no benchmark result and has no verdict. The harness was explicitly
synced and byte-checked on the worker; only the subsequent invocation whose
process self-reported `87ce0065...` was admitted.

## Results

| size | FP p50 | pandas fastest p50 | pandas/FP ratio | FP A/A median CI | pandas A/A median CI | effect / required | verdict |
|---:|---:|---:|---:|---|---|---:|---|
| 1M | 26.223 ms | 41.446 ms | **1.580x** | [0.972114, 1.014791] | [0.925429, 1.142554] | 0.45773268 / 0.26653188 | **FASTER** |
| 10M | 265.554 ms | 447.255 ms | **1.684x** | [0.971938, 1.058421] | [0.961573, 1.057509] | 0.52130954 / 0.11355576 | **FASTER** |

The two-row geomean is 1.631x. Absolute median time saved grows from
15.222 ms to 181.701 ms, while the ratio improves 6.6% from 1M to 10M. This
is both a live-incumbent win and a modest ratio-amplifying Class-1 result on
this ELF.

CV was FP/pandas 7.24%/21.78% at 1M and 9.70%/13.73% at 10M. It did not
decide either row. The median effect at each size independently cleared the
predeclared median-CI threshold.

## Validation

- Same-worker pandas route screen and exact Series equality: PASS.
- Raw schema-v4 contract self-check: PASS.
- Worker/local source and raw-artifact byte identity: PASS.
- In-process/direct-worker ELF SHA-256 identity: PASS.
- Focused `cargo test --locked -p fp-frame dt_month_name --lib`: PASS, 1/1,
  strict remote.
- `cargo check --locked --workspace --all-targets`: PASS, strict remote.
- `cargo clippy --locked -p fp-bench --all-targets --no-deps -- -D warnings`:
  PASS, strict remote.
- `cargo fmt --check -p fp-bench`: PASS.
- Python harness bytecode compilation: PASS.
- `git diff --check`: PASS.
- Focused UBS scan: no focused defect. The scanner reproduced the established
  fixed-executable/argv subprocess false positive and pre-existing JSON/ruff
  findings outside this hunk.

## Decision and concrete retry predicates

Keep the `dt.month_name` incumbent-win classification and retain the fastest
pandas route as permanent measurement coverage. Historical ratios against
direct `Series.dt.month_name()` are weaker-incumbent diagnostics, not current
competitive claims.

Re-open only when at least one of these concrete predicates holds:

- The FrankenPandas month-name kernel, datetime representation, Utf8 output,
  allocator, compiler, worker ISA, fixture, pandas or NumPy artifact, harness
  source, or executing ELF changes.
- A fresh same-worker pandas screen finds a faster route that produces an
  exactly equal complete Series.
- A fresh canonical run with one self-identified ELF and 25 alternating A/A
  pairs per engine makes either 1M or 10M non-decidable or places its ratio at
  or below 1.0 outside the combined null interval.

Any revalidation still requires the fastest live pandas arm in the same
invocation and median-CI admission at both sizes. Do not use direct
`.dt.month_name()`, `.dt.strftime("%B")`, or the slower `Series.dt.month`
route as a headline incumbent unless a new screen makes it fastest.

Raw evidence:
`artifacts/bench/proud_lane_m_dt_month_name_1m_10m_20260728.json`.
