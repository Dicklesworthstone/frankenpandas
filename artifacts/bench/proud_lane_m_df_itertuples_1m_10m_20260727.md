# Lane M large-N row-tuple incumbent adjudication — 2026-07-27

This artifact completes the admission contract for the earlier directional
`df_itertuples` row and records the 10M result without promoting an unstable
number into a claim.

## Admitted result

At 1M rows x 10 Float64 columns, FrankenPandas was:

- **16.102x faster** than the exact pandas 2.2.3 API,
  `list(df.itertuples())`; and
- **7.562x faster** than `df.to_records(index=True).tolist()`, the fastest of
  four task-equivalent pandas idioms screened before the run.

The second ratio is the conservative campaign headline. Both engines fully
materialize one row product containing the index and all ten values. The
FrankenPandas arm calls `DataFrame::itertuples`; no production implementation
changed in this work.

The 10M ratios were numerically 3.620x and 6.080x, but both are
`NULL_UNDECIDABLE`. FrankenPandas' 10M A/A intervals span the claimed effects,
so neither number is an admitted competitive claim.

**Campaign result class:** `incumbent-win`.

## Protocol and identity

- Worker: RCH alias `ovh-a` for the complete invocation.
- Build/run route: `RCH_WORKER=ovh-a RCH_REQUIRE_REMOTE=1
  env -u CARGO_TARGET_DIR rch exec -- cargo run --locked --profile
  release-perf -p fp-bench -- --remote-python-harness ...`.
- Shared invocation:
  `vs-pandas-20260728T025312.250425Z-pid1927317`.
- pandas: version 2.2.3, imported on the same worker and hashed as a
  deterministic installed-distribution content tree.
- Samples: 50 timed samples and 25 alternating A/A pairs per engine and row.
- Decision: deterministic 10,000-resample bootstrap 95% CI over each A/A
  median; the required log effect is twice the larger null-CI log half-width.
- CV: recorded as provenance only and never used as an acceptance gate.

**Executing ELF SHA-256 (self-reported by process):**
`bench_elf_sha256=1ef0c1e07a5b0b5d57904b255e41e7306d0f277a2c12fa8d4ccff774848c623c
(70,209,136 bytes)
/data/projects/frankenpandas/.rch-target-ovh-a-pool-bc445989bdf88102bcbc62abd4347d69/release-perf/fp-bench`

**Legacy incumbent arm (same invocation):**
`name=pandas version=2.2.3
artifact_sha256=fb69f90acac18b871bb69f5eab56bea198b17692c5045de29eed608132a959c9
invocation_id=vs-pandas-20260728T025312.250425Z-pid1927317
measured_ratio=7.562x`

The imported pandas artifact contained 2,922 files and 70,709,779 bytes. Its
interpreter identity was
`efb29ce53d36ebaeee80e3aa44fd6c7f9d71bbded5fe1665240b2ed8ecaeee0e`
(`/usr/bin/python3.13`, 6,894,448 bytes).

## Median-CI adjudication

| incumbent workload | size | FP p50 | pandas p50 | pandas/FP | FP A/A median and 95% CI | pandas A/A median and 95% CI | claim log effect | required log effect | verdict |
|---|---:|---:|---:|---:|---|---|---:|---:|---|
| exact `list(df.itertuples())` | 1M | 74.978 ms | 1,207.293 ms | **16.102x** | 0.999747 [0.998815, 1.002631] | 1.034233 [0.964552, 1.042417] | 2.77893859 | 0.08308420 | **FASTER** |
| fastest screened row-tuple idiom | 1M | 75.095 ms | 567.903 ms | **7.562x** | 1.000891 [0.997740, 1.002731] | 1.000053 [0.991804, 1.002277] | 2.02319795 | 0.01645864 | **FASTER** |
| exact `list(df.itertuples())` | 10M | 4,204.863 ms | 15,222.475 ms | 3.620x | 0.137812 [0.125259, 7.651503] | 0.976258 [0.961040, 1.027252] | 1.28653122 | 4.15474274 | `NULL_UNDECIDABLE` |
| fastest screened row-tuple idiom | 10M | 1,011.189 ms | 6,148.378 ms | 6.080x | 0.965300 [0.213035, 4.862635] | 1.004337 [0.999103, 1.009067] | 1.80506167 | 3.16316085 | `NULL_UNDECIDABLE` |

**A/A null control (same invocation):** 25 alternating pairs per engine and
row. For the conservative admitted 1M row, FP median=1.000891 with 95%
median CI=[0.997740, 1.002731], while pandas median=1.000053 with 95%
median CI=[0.991804, 1.002277].

**Median-CI decision:** the conservative 1M median effect had
claim_log_effect=2.02319795 and cleared the required effect=0.01645864. At
10M, required effects of 4.15474274 and 3.16316085 exceeded the respective
claim effects, so both rows stayed inside the median-CI null floor.

**CV role:** provenance only; CV had no vote.

## Route and behavior audit

The exact pandas arm executes `len(list(df.itertuples()))`. The conservative
arm executes `len(df.to_records(index=True).tolist())`. The pre-run selection
screen also checked `itertuples(name=None)` and
`to_numpy()` plus `map(tuple)`; `to_records(...).tolist()` was fastest on that
screen. The authoritative conservative timing above nevertheless ran that
selected incumbent inside the same invocation as the FrankenPandas arm.

Both FrankenPandas workload names route to `df.itertuples()` after the common
10M frame constructor. `DataFrame::itertuples` returns one
`(IndexLabel, Vec<Scalar>)` per row, preserving index order and column order.
Existing unit and golden tests cover the observable row values. This commit
changes benchmark routing and provenance only, not that production method.

Strict-remote validation passed `cargo check --locked --workspace
--all-targets`, all 3 `fp-bench` harness-contract tests, both filtered
`DataFrame::itertuples` unit/golden tests, and focused
`cargo clippy -p fp-bench --all-targets --no-deps -- -D warnings`.
Workspace-wide clippy remains blocked by the pre-existing `fp-columnar` lint
backlog before it reaches `fp-bench`.

The first attempted 10M invocation was discarded before analysis because the
Python harness generated 10M rows while the Rust size router silently fell
back to 100k. The admitted invocation ran only after adding an exact
`10M -> (10_000_000, 10)` mapping and a regression test. In the admitted JSON,
the two independent 10M FrankenPandas p50s are 13.47x and 56.08x their
corresponding 1M p50s, proving the corrected route did not use the old 100k
fallback.

## Retry predicate

Do not quote either 10M ratio from this artifact. Reopen 10M row
materialization only after the same-invocation supervisor runs every timed
arm in a fresh child process, records peak RSS and major faults, keeps the
same-worker executable and pandas identities, and reduces the combined A/A
required log effect to at most 0.20. The rerun must again include both the
exact pandas API and the independently selected task-equivalent incumbent
arm. Until then, the admitted campaign result is the conservative 7.562x
incumbent win at 1M.

Canonical machine-readable evidence:
`artifacts/bench/proud_lane_m_df_itertuples_1m_10m_20260727.json`
(SHA-256
`dd1436e016ce09172f4c4a100deedb07cf3b7e266b62ea25e7efaa7b284212a0`).
