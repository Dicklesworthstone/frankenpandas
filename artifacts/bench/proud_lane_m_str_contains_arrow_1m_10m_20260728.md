# Lane M literal `str.contains` strongest-incumbent gate

Date: 2026-07-28 America/New_York

Bead: `br-frankenpandas-gfeu1`

Result: **REJECT** the competitive claim. FrankenPandas reached 0.844x pandas
at 1M and 0.665x at 10M. Both losses are decidable under the median-CI gate,
and the disadvantage grows with scale.

## Incumbent admission and semantic proof

The fixture is the existing `fp-bench` string `name` column:
`item_{i:010d}` for `i` in `0..n`. Both engines search for the literal
substring `"5"` over an all-valid Series. pandas uses
`Series.str.contains("5", regex=False)`; FrankenPandas uses
`Series::str().contains("5")`.

A same-worker 1M route screen on `vmi1264463` compared all relevant pandas
2.2.3 string backends on those exact names:

| pandas backend | p50 | output |
|---|---:|---|
| object | 177.904 ms | equal |
| `string[python]` | 156.607 ms | equal |
| `string[pyarrow]` | **41.948 ms** | equal |

All three output arrays were exactly equal and contained 468,559 true
elements. Only the fastest, `string[pyarrow]`, advanced to the canonical
gate. The object and Python-string rows are route-screen diagnostics, not
competitive incumbents.

The strict-remote focused `fp-frame` `str_contains` test filter passed all
13 tests before the gate. Those tests cover literal matching, whole-buffer
scan boundaries, null propagation, `regex=False` metacharacter treatment,
case options, invalid regex handling, and golden output. Production source
did not change.

Behavior-preservation checklist:

- Values: identical zero-padded names on both engines.
- Search semantics: literal substring, case-sensitive, all valid.
- Ordering: input order preserved.
- Null handling: outside this all-valid fixture; covered by focused tests.
- Regex behavior: explicitly disabled on pandas and not invoked on FP.
- RNG: none.

The CLI dtype is the harness's established nominal `float64` routing tag for
string workloads. The generic numeric frame supplies only `len(df)` and is
outside timing; both timed arms construct the exact string fixture above.

## Measurement contract

- Worker: `vmi1264463` (`38.242.209.154`).
- Disk guard: 337 GiB free before the successful Cargo invocation, above the
  120 GiB floor.
- Strict-remote command:

  ```text
  RCH_WORKER=vmi1264463 RCH_REQUIRE_REMOTE=1 env -u CARGO_TARGET_DIR rch exec -- cargo run --locked --profile release-perf -p fp-bench -- --remote-python-harness --category strings --sizes 1M,10M --dtypes float64 --workloads str_contains_arrow --output artifacts/bench/proud_lane_m_str_contains_arrow_1m_10m_20260728.json --json-stdout
  ```

- Shared invocation ID:
  `vs-pandas-20260729T043315.046949Z-pid3676335`.
- In-process FP ELF SHA-256:
  `ad25a86447134d5fac336cdb0d9af3def77adb2bd921e3854aaaa9b0b58256b1`
  (70,379,784 bytes). A direct worker hash matched.
- Rust benchmark source SHA-256:
  `5ba72d89a3bf5888b635cc6c0937213bd03ae536536b6467a3058656548f1484`.
  The worker and local hashes matched.
- Python harness source SHA-256:
  `a3968444b266656cf8cdbbf142b98445e486f39634251e6ff6131c7b973b19c1`
  (67,595 bytes). The worker, local, and in-process reports matched.
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
  `78650f5cca479b7352b804c14f4263a823923dde2796ea2558868eabfaea5da5`.

## Results

| size | FP p50 | pandas Arrow p50 | FP/pandas ratio | FP A/A median CI | pandas A/A median CI | effect / required | verdict |
|---:|---:|---:|---:|---|---|---:|---|
| 1M | 50.187 ms | 42.342 ms | **0.844x** | [0.921988, 1.076340] | [0.937536, 1.006525] | 0.16997788 / 0.16244565 | **SLOWER** |
| 10M | 630.038 ms | 418.686 ms | **0.665x** | [0.985473, 1.111841] | [0.955177, 1.028445] | 0.40865758 / 0.21203502 | **SLOWER** |

The two-row geomean is 0.749x, meaning pandas is 1.335x faster on the
geomean. The FP/pandas ratio falls 21.2% from 1M to 10M, while the absolute
median deficit grows from 7.845 ms to 211.351 ms. This is the opposite of a
scale-amplified Class-1 win.

CV was high (FP/pandas 30.81%/16.37% at 1M and 23.79%/15.43% at 10M), but
it did not decide either row. Both effects independently cleared the
predeclared median-CI threshold.

## Decision and retry predicates

Reject any claim that literal `str.contains` currently beats the strongest
pandas incumbent at scale. Retain the live Arrow incumbent arm so future
source changes cannot hide behind an object-dtype comparison.

Re-open only when at least one of these concrete predicates holds:

- A profile of this exact 10M workload names a non-zero-self FP frame, gives
  its self-time, and computes an Amdahl ceiling for a production lever capable
  of removing at least 33.6% of FP median time (the minimum needed for parity
  with this pandas artifact).
- The FP literal-contains implementation, contiguous-Utf8 representation,
  allocator, compiler, worker ISA, fixture, pandas artifact, or pyarrow
  artifact changes.
- A pandas/pyarrow version change triggers a fresh same-worker backend screen;
  only the new fastest semantically identical backend may advance.

Do not retry by switching pandas to object or `string[python]`; those weaker
arms were already screened out. A future result still requires one
self-identified ELF, the live fastest pandas arm in the same invocation,
alternating A/A controls, and median-CI admission at both sizes.

Raw evidence:
`artifacts/bench/proud_lane_m_str_contains_arrow_1m_10m_20260728.json`.
