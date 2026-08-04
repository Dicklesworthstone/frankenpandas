# Lane M `dt.day_name` strongest-incumbent gate

Date: 2026-07-28 America/New_York

Bead: `br-frankenpandas-mdhgp`

Result: **KEEP** the competitive claim. FrankenPandas reached 3.463x pandas
at 1M and 3.716x at 10M. Both wins are independently decidable under the
median-CI gate.

**Campaign result class:** `incumbent-win`.

## Incumbent admission and semantic proof

Both engines receive the same all-valid `datetime64[ns]` sequence:

```text
base = 946684800000000000  # 2000-01-01T00:00:00
value[i] = base + i * 600000000000  # 600 seconds
```

Population remains outside timing. A same-worker pandas 2.2.3 route screen
on the exact 1M-row fixture compared three complete Series-producing routes:

| pandas route | p50 | output |
|---|---:|---|
| `Series.dt.day_name()` | 563.805 ms | equal |
| `Series.dt.strftime("%A")` | 7,465.375 ms | equal |
| `Series.dt.dayofweek` + NumPy name gather + `Series(...)` | **54.286 ms** | equal |

All three results were exactly equal, including the object dtype, Series
index and name. They contained seven unique English weekday names; the first
and last 1M-row values were both `Saturday`. Only the fastest route advanced
to the canonical gate:

```text
codes = series.dt.dayofweek.to_numpy(copy=False)
pd.Series(day_names[codes], index=series.index, name=series.name)
```

This is a live pandas arm, not a NumPy-only output: pandas performs the
datetime accessor and the timed region constructs the complete result
Series. The weekday-name table and datetime population remain outside timing
on both engines.

The FrankenPandas arm executes `series.dt().day_name()`. The strict-remote
focused `fp-frame` `dt_day_name` test passed before measurement. Production
source did not change.

Behavior-preservation checklist:

- Values: identical English weekday names for the same nanosecond timestamps.
- Order and cardinality: one result per input row in original order.
- Result contract: object/string Series with the same index and name.
- Null behavior: outside this all-valid fixture; retained by the existing
  production implementation and focused test.
- RNG: none.

## Measurement contract

- Worker: `vmi1264463` (`38.242.209.154`).
- Disk guard: 361 GiB free before the Cargo invocation, above the 120 GiB
  floor.
- Strict-remote command:

  ```text
  RCH_WORKER=vmi1264463 RCH_REQUIRE_REMOTE=1 env -u CARGO_TARGET_DIR rch exec -- cargo run --locked --profile release-perf -p fp-bench -- --remote-python-harness --category datetime --sizes 1M,10M --dtypes datetime64 --workloads dt_day_name --output artifacts/bench/proud_lane_m_dt_day_name_1m_10m_20260728.json --json-stdout
  ```

- Shared invocation ID:
  `vs-pandas-20260729T053143.056179Z-pid3783239`.
- In-process FP ELF SHA-256:
  `0b212606e7b27a180f4d01e74f12965aa67c59a2cef8f9b3d3de2410629766cd`
  (70,379,824 bytes). A direct worker hash matched.
- Rust benchmark source SHA-256:
  `bf155d88b2fc76b163b021375adb9b5674ba47ed4792253d1512285a47902cc4`.
  The worker and local hashes matched.
- Python harness source SHA-256:
  `251ac27c48c2f484da72af4d0ab58e1e22c310b5b6c707879bbba5df267b4d75`
  (69,242 bytes). The worker, local, and in-process reports matched.
- Python executable SHA-256:
  `efb29ce53d36ebaeee80e3aa44fd6c7f9d71bbded5fe1665240b2ed8ecaeee0e`.
- pandas 2.2.3 content-tree SHA-256:
  `051be80fe43b4e0be4e04af314c42db966950eb877b0634d482099f42535e9bb`.
- Every engine and size ran 25 alternating A/A pairs in the same invocation
  as its A/B comparison.
- Decision gate: twice the larger FP/pandas bootstrap-median 95% CI log
  half-width. CV is provenance only and had no vote.
- Raw JSON SHA-256:
  `ac6e425f512f86d756a30281b8aec4ecfca999216243fc935213b1c62d434913`.

## Results

| size | FP p50 | pandas fastest p50 | pandas/FP ratio | FP A/A median CI | pandas A/A median CI | effect / required | verdict |
|---:|---:|---:|---:|---|---|---:|---|
| 1M | 13.204 ms | 45.721 ms | **3.463x** | [0.954469, 1.041674] | [0.990359, 1.056384] | 1.24205526 / 0.10970379 | **FASTER** |
| 10M | 130.697 ms | 485.621 ms | **3.716x** | [0.975424, 1.110409] | [0.998803, 1.054907] | 1.31254780 / 0.20945764 | **FASTER** |

The two-row geomean is 3.587x. Absolute median time saved grows from
32.517 ms to 354.924 ms, while the ratio improves 7.3% from 1M to 10M. This
is both a decisive live-incumbent win and a modest ratio-amplifying Class-1
result on this ELF.

CV was FP/pandas 98.51%/8.17% at 1M and 276.57%/7.27% at 10M. It did not
decide either row. The median effect at each size independently cleared the
predeclared median-CI threshold by a wide margin.

## Validation

- Same-worker pandas route screen and exact Series equality: PASS.
- Raw schema-v4 contract self-check: PASS.
- Worker/local source and raw-artifact byte identity: PASS.
- In-process/direct-worker ELF SHA-256 identity: PASS.
- Focused `cargo test --locked -p fp-frame dt_day_name --lib`: PASS, 1/1,
  strict remote.
- `cargo check --locked --workspace --all-targets`: PASS, strict remote.
- `cargo clippy --locked -p fp-bench --all-targets --no-deps -- -D warnings`:
  PASS, strict remote.
- `cargo fmt --check -p fp-bench`: PASS.
- Python harness bytecode compilation: PASS.
- `git diff --check`: PASS.
- Focused UBS scan: no focused defect. The scanner reproduced the established
  fixed-executable/argv subprocess false positive and pre-existing JSON/ruff
  findings outside this hunk; its only new-hunk item was informational
  attribute-chain output for the locally constructed datetime Series.

## Decision and concrete retry predicates

Keep the `dt.day_name` incumbent-win classification and retain the fastest
pandas route as permanent measurement coverage. Historical ratios against
direct `Series.dt.day_name()` are weaker-incumbent diagnostics, not current
competitive claims.

Re-open only when at least one of these concrete predicates holds:

- The FrankenPandas day-name kernel, datetime representation, Utf8 output,
  allocator, compiler, worker ISA, fixture, pandas artifact, NumPy artifact,
  harness source, or executing ELF changes.
- A fresh same-worker pandas screen finds a faster route that produces an
  exactly equal complete Series.
- A fresh canonical run with one self-identified ELF and 25 alternating A/A
  pairs per engine makes either 1M or 10M non-decidable or places its ratio at
  or below 1.0 outside the combined null interval.

Any revalidation still requires the fastest live pandas arm in the same
invocation and median-CI admission at both sizes. Do not use the direct
`.dt.day_name()` or `.dt.strftime("%A")` routes as headline incumbents unless
a new screen makes one of them fastest.

Raw evidence:
`artifacts/bench/proud_lane_m_dt_day_name_1m_10m_20260728.json`.
