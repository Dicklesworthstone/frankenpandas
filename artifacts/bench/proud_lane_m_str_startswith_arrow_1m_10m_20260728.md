# Lane M `str.startswith` strongest-incumbent gate

Date: 2026-07-28 America/New_York

Bead: `br-frankenpandas-byv7n`

Result: **REJECT** the competitive claim. FrankenPandas reached 0.406x pandas
at 1M and 0.434x at 10M. Both losses are decidable under the median-CI gate.

## Incumbent admission and semantic proof

The fixture is the existing `fp-bench` string `name` column:
`item_{i:010d}` for `i` in `0..n`. Both engines test the case-sensitive
literal prefix `"item"` over an all-valid Series. pandas uses
`Series.str.startswith("item")`; FrankenPandas uses
`Series::str().startswith("item")`.

A same-worker 1M route screen on `vmi1264463` compared all relevant pandas
2.2.3 string backends:

| pandas backend | p50 | true count | output |
|---|---:|---:|---|
| object | 193.081 ms | 1,000,000 | equal |
| `string[python]` | 146.044 ms | 1,000,000 | equal |
| `string[pyarrow]` | **10.394 ms** | 1,000,000 | equal |

All three output arrays were exactly equal. Only the fastest,
`string[pyarrow]`, advanced to the canonical gate. The object and
Python-string rows are route-screen diagnostics, not competitive incumbents.

The strict-remote focused `fp-frame` `str_startswith` test filter passed all
6 tests before the gate. Those tests cover the literal prefix, tuple-prefix
matching, null propagation, explicit null fill, and golden output. Production
source did not change.

Behavior-preservation checklist:

- Values: identical zero-padded names on both engines.
- Prefix semantics: literal, case-sensitive, all valid.
- Result: one true boolean per input row in original order.
- Null behavior: outside this all-valid fixture; covered by focused tests.
- RNG: none.

The CLI dtype is the harness's established nominal `float64` routing tag for
string workloads. The generic numeric frame supplies only `len(df)` and is
outside timing; both timed arms construct the exact string fixture above.

## Measurement contract

- Worker: `vmi1264463` (`38.242.209.154`).
- Disk guard: 309 GiB free before the Cargo invocation, above the 120 GiB
  floor.
- Strict-remote command:

  ```text
  RCH_WORKER=vmi1264463 RCH_REQUIRE_REMOTE=1 env -u CARGO_TARGET_DIR rch exec -- cargo run --locked --profile release-perf -p fp-bench -- --remote-python-harness --category strings --sizes 1M,10M --dtypes float64 --workloads str_startswith_arrow --output artifacts/bench/proud_lane_m_str_startswith_arrow_1m_10m_20260728.json --json-stdout
  ```

- Shared invocation ID:
  `vs-pandas-20260729T050418.942527Z-pid3741365`.
- In-process FP ELF SHA-256:
  `64a5bcccd668d2c05cf76f0f25458cf6d444c719f7347079ce1ddf63677f1ae0`
  (70,379,840 bytes). A direct worker hash matched.
- Rust benchmark source SHA-256:
  `bf155d88b2fc76b163b021375adb9b5674ba47ed4792253d1512285a47902cc4`.
  The worker and local hashes matched.
- Python harness source SHA-256:
  `798465900ee2052aba24b7d523594e7226d1df19f448761ca776360ff6a2faa7`
  (68,211 bytes). The worker, local, and in-process reports matched.
- Python executable SHA-256:
  `efb29ce53d36ebaeee80e3aa44fd6c7f9d71bbded5fe1665240b2ed8ecaeee0e`.
- pandas 2.2.3 content-tree SHA-256:
  `051be80fe43b4e0be4e04af314c42db966950eb877b0634d482099f42535e9bb`.
- pyarrow 24.0.0 content-tree SHA-256:
  `2e701e78b2e69a481b6e901b584db29c4151221f59568dcb7cde7f036bca5f17`.
- Every engine and size ran 25 alternating A/A pairs in the same invocation
  as its A/B comparison.
- Decision gate: twice the larger FP/pandas bootstrap-median 95% CI log
  half-width. CV is provenance only and had no vote.
- Raw JSON SHA-256:
  `a14dce9be27576dfeb90ac7f8afb1a3a81077256e240a27471e221f394f1e6d6`.

## Results

| size | FP p50 | pandas Arrow p50 | FP/pandas ratio | FP A/A median CI | pandas A/A median CI | effect / required | verdict |
|---:|---:|---:|---:|---|---|---:|---|
| 1M | 25.440 ms | 10.329 ms | **0.406x** | [0.954257, 1.090087] | [0.963220, 1.041711] | 0.90132742 / 0.17251561 | **SLOWER** |
| 10M | 250.332 ms | 108.572 ms | **0.434x** | [0.955747, 1.023732] | [0.855253, 0.996646] | 0.83537229 / 0.31271556 | **SLOWER** |

The two-row geomean is 0.420x, meaning pandas is 2.382x faster on the
geomean. The ratio improves 6.9% from 1M to 10M but remains a large,
decidable loss; the absolute median deficit grows from 15.110 ms to
141.760 ms.

CV was FP/pandas 12.78%/12.98% at 1M and 10.37%/21.28% at 10M. It did not
decide either row. Both effects independently cleared the predeclared
median-CI threshold.

## Decision and retry predicates

Reject any claim that `str.startswith` currently beats the strongest pandas
incumbent at scale. Retain the live Arrow arm so future source changes cannot
hide behind an object-dtype comparison.

Re-open only when at least one of these concrete predicates holds:

- A profile of this exact 10M workload names a non-zero-self FP frame, gives
  its self-time, and computes an Amdahl ceiling for a production lever capable
  of removing at least 56.7% of FP median time (the minimum needed for parity
  with this pandas artifact).
- The FP prefix implementation, contiguous-Utf8 representation, Bool output,
  allocator, compiler, worker ISA, fixture, pandas artifact, or pyarrow
  artifact changes.
- A pandas/pyarrow version change triggers a fresh same-worker backend screen;
  only the new fastest semantically identical backend may advance.

Do not retry by switching pandas to object or `string[python]`; those weaker
arms were already screened out. A future result still requires one
self-identified ELF, the live fastest pandas arm in the same invocation,
alternating A/A controls, and median-CI admission at both sizes.

Raw evidence:
`artifacts/bench/proud_lane_m_str_startswith_arrow_1m_10m_20260728.json`.
