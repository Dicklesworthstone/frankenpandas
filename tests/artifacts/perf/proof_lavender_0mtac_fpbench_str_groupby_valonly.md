# br-frankenpandas-0mtac proof: fp-bench string groupby-sum value selection

## Target

Bead `br-frankenpandas-0mtac` was filed from the vs-pandas strings harness for
`df.groupby("key")["val"].sum()` on the deterministic `_build_str_frame`
workload: 1M rows, 1000 distinct `g0000..g0999` string keys, a separate
`name` string column, and one Float64 `val` column.

The pandas oracle path in `benches/vs_pandas_harness.py` is:

```python
f = _build_str_frame(len(df))
return time_operation(lambda: f.groupby("key")["val"].sum())
```

## Lever

One lever: make `fp-bench strings/str_groupby_sum` select only `key,val` before
calling `groupby(&["key"]).sum()`.

Before this change, the FP side timed `frame.groupby(&["key"]).sum()` over all
non-key columns, which included the unrelated Utf8 `name` column. That was not
the pandas oracle workload and it also bypassed the dense string-key Float64
sum path that the bead intended to measure.

Production groupby implementation code was not changed.

## Timing Evidence

Baseline command:

```bash
env -u CARGO_TARGET_DIR RCH_VERBOSE=1 rch exec -- \
  cargo run --profile release-perf -p fp-bench -- \
  --category strings --workload str_groupby_sum --size 1M --dtype float64 --json
```

Baseline artifact:
`tests/artifacts/perf/lavender_0mtac_base_fp_bench_1m_str_groupby_sum.txt`

Baseline JSON SHA256:
`945e515585ceafcb94dc18fd6186fcd111b0515a9530ebae44cceae099b5ace7`

Baseline stats:

```json
{"mean_us":63533.959400000014,"median_us":62249.691000000006,"min_us":54601.574,"max_us":78062.723,"n":25}
```

Candidate command:

```bash
env -u CARGO_TARGET_DIR RCH_VERBOSE=1 rch exec -- \
  cargo run --profile release-perf -p fp-bench -- \
  --category strings --workload str_groupby_sum --size 1M --dtype float64 --json
```

Candidate artifact:
`tests/artifacts/perf/lavender_0mtac_fpbench_valonly_candidate_1m_str_groupby_sum.txt`

Candidate JSON SHA256:
`eae32963af4212b0e958e8bdc3a2acf517a430f0692e1680817d426ec2f2eecf`

Candidate stats:

```json
{"mean_us":6982.682999999999,"median_us":6671.103999999999,"min_us":6135.9800000000005,"max_us":9066.243,"n":25}
```

Observed same-harness speedup:

- Mean: `63533.9594 us -> 6982.6830 us` = `9.098789x`
- Median: `62249.6910 us -> 6671.1040 us` = `9.331243x`

Post-validation direct binary smoke:

- Artifact: `tests/artifacts/perf/lavender_0mtac_direct_binary_after_checks.txt`
- JSON SHA256: `aca854691215158addcc9c879703d43a1c74f6eca2b1b778a62affeb113521a7`
- Stats: `mean 6290.43844 us`, `median 6302.533 us`, `n=25`

## Oracle Context

After unsetting the ambient `CARGO_TARGET_DIR=/data/tmp/cargo-target` so the
harness used the rebuilt in-tree `target/release-perf/fp-bench`, the existing
vs-pandas harness produced:

- Artifact: `tests/artifacts/perf/lavender_0mtac_vs_pandas_strings_1m.json`
- `str_groupby_sum` FP p50: `6372.31 us`, CV `3.39%`, valid
- `str_groupby_sum` pandas p50: `27483.17 us`, CV `11.42%`, invalid under the
  harness 5% CV rule
- Harness verdict: `DROPPED_HIGH_CV`

This oracle run is supporting context only because pandas exceeded the harness
CV threshold. The keep gate is the direct same-command before/after timing
above.

## Isomorphism Proof

- Ordering: unchanged. `DataFrameGroupBy::sum()` still emits groups in the
  existing sorted-key order. The lever only removes the unrelated `name` value
  column from the timed operation to match pandas' selected `["val"]` workload.
- Tie-breaking: unchanged. No sort comparator, rank method, or duplicate
  policy changed.
- Floating point: unchanged for the target value stream. The summed Float64
  values and row visitation order are identical to the `val` column values that
  were already present in the original frame. The removed `name` column had no
  pandas-observable role in `f.groupby("key")["val"].sum()`.
- RNG: none. `build_str_frame` and the proof builder are deterministic from row
  number alone.
- Null/NaN: unchanged. This workload's `val` column is all-valid finite
  Float64 values.
- Production behavior: unchanged. Only `fp-bench` workload selection and the
  golden proof harness were edited.

Golden command:

```bash
env -u CARGO_TARGET_DIR RCH_VERBOSE=1 rch exec -- \
  cargo run --profile release-perf -p fp-conformance --example perf_profile -- \
  golden str_groupby_sum_fpbench_valonly 5000
```

Golden artifact:
`tests/artifacts/perf/lavender_0mtac_valonly_golden_5000.txt`

Golden SHA256:
`a322aff6215a3d47fd08c8052c21e7f37223d9cdcc92b2d52c96e3d901ff8f19`

The golden dump has `nrows=1000`, sorted labels `g0000..g0999`, and one
Float64 `val` column.

## Validation

All validation commands were run after the candidate change:

- `cargo fmt -p fp-bench -p fp-conformance --check`: pass
- `rch exec -- cargo check -p fp-bench --all-targets`: pass
- `rch exec -- cargo check -p fp-conformance --example perf_profile`: pass
- `rch exec -- cargo clippy -p fp-bench --all-targets -- -D warnings`: pass
- `rch exec -- cargo clippy -p fp-conformance --example perf_profile -- -D warnings`: pass
- `ubs crates/fp-bench/src/main.rs crates/fp-conformance/examples/perf_profile.rs`: exit 0; no critical issues; existing harness-style warnings only

Sampling profiler note: `perf`/`samply` sampling was blocked by host
`perf_event_paranoid=4`, recorded in the local route artifacts. Timing evidence
is therefore the keep evidence for this bead.

## Score

Impact: `9.10` (same-command mean speedup)
Confidence: `0.95` (same harness, golden SHA, oracle source match)
Effort: `1.5`

Score: `9.10 * 0.95 / 1.5 = 5.76`, which is above the `2.0` keep threshold.
