# br-frankenpandas-uza04.90 Pass 1 Baseline/Profile Witness

Date: 2026-06-13
Worktree: `/data/projects/.scratch/frankenpandas-uza0489-orangepeak-20260613`
Head: `8e4eee3d784e2053caa76d281833b31151853bdd`
Scope: evidence-only; no runtime source code edits.

## Build

Command:

```text
RUSTFLAGS='-C force-frame-pointers=yes' CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0490-base rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Evidence: `tests/artifacts/perf/uza0490_base_build_perf_profile.txt`

- RCH selected worker: `vmi1227854` at `root@109.123.245.77`
- RCH executed remote cargo build: yes
- RCH target rewrite: `/data/projects/.scratch/cargo-target-orangepeak-uza0490-base` to `.rch-target-vmi1227854-job-29884606035526637-1781319241345789807-0`
- Result: exit 0, `[RCH] remote vmi1227854 (265.3s)`
- Binary: `/data/projects/.scratch/cargo-target-orangepeak-uza0490-base/release-perf/examples/perf_profile`
- Binary BuildID: `4ad2b0dbc3e4b7ae989cc84652fb47fe2d273dab`

## Golden Outputs

Generated with:

```text
/data/projects/.scratch/cargo-target-orangepeak-uza0490-base/release-perf/examples/perf_profile golden df_kendall <n>
```

SHA256:

```text
acf366c266b66f8497fb55734ed1b4ec40952a7c014f43db262c7c1e625e15e1  tests/artifacts/perf/uza0490_base_golden_df_kendall_2000.txt
031978ba431260b942dd36d9be055064a7453118a7c00888c35747644d33d99e  tests/artifacts/perf/uza0490_base_golden_df_kendall_5000.txt
f164fa86300fbea93e46aa99a5e0f7413fa8c0baf2ee49c6a689ed92652a4c3b  tests/artifacts/perf/uza0490_base_golden_df_kendall_20000.txt
```

Checksum verification artifact: `tests/artifacts/perf/uza0490_base_golden_check.txt`

## Hyperfine Baseline

RCH wrapper note: both hyperfine commands were invoked via `rch exec --`, but RCH warned `exec called with non-compilation command` and did not report a remote worker line for the benchmark runs. Treat these as local-wrapper baseline timings using the RCH-built binary.

`df_kendall 50000 1`

- JSON: `tests/artifacts/perf/uza0490_base_hyperfine_df_kendall_50000x1.json`
- Transcript: `tests/artifacts/perf/uza0490_base_hyperfine_df_kendall_50000x1.txt`
- Mean: 139.704 ms
- Stddev: 16.760 ms
- Median: 135.217 ms
- Min: 120.915 ms
- Max: 177.645 ms
- p50 from sorted samples: 137.388 ms
- p95 from sorted samples: 177.645 ms
- p99 from sorted samples: 177.645 ms
- User: 908.596 ms
- System: 61.159 ms

`df_kendall 200000 1`

- JSON: `tests/artifacts/perf/uza0490_base_hyperfine_df_kendall_200000x1.json`
- Transcript: `tests/artifacts/perf/uza0490_base_hyperfine_df_kendall_200000x1.txt`
- Mean: 589.332 ms
- Stddev: 12.319 ms
- Median: 588.837 ms
- Min: 569.348 ms
- Max: 606.867 ms
- p50 from sorted samples: 589.219 ms
- p95 from sorted samples: 606.867 ms
- p99 from sorted samples: 606.867 ms
- User: 5165.453 ms
- System: 254.401 ms

## Profile/Stat Witness

`perf record` command:

```text
rch exec -- perf record -g --call-graph fp -o tests/artifacts/perf/uza0490_base_perf_df_kendall_200000x1.data -- /data/projects/.scratch/cargo-target-orangepeak-uza0490-base/release-perf/examples/perf_profile df_kendall 200000 1
```

Result: blocked, exit 255. Transcript: `tests/artifacts/perf/uza0490_base_perf_record_df_kendall_200000x1.txt`.

Blockage:

```text
Access to performance monitoring and observability operations is limited.
perf_event_paranoid setting is 4
```

The generated data file is zero bytes: `tests/artifacts/perf/uza0490_base_perf_df_kendall_200000x1.data`.

`perf stat -d -r 3` was also blocked with the same `perf_event_paranoid=4` message. Transcript: `tests/artifacts/perf/uza0490_base_perf_stat_df_kendall_200000x1.txt`.

Fallback resource stat:

- Artifact: `tests/artifacts/perf/uza0490_base_time_v_df_kendall_200000x1.txt`
- Wall time: 0.54 s
- User time: 4.88 s
- System time: 0.22 s
- CPU: 931%
- Max RSS: 349708 KB
- Minor page faults: 99663
- Voluntary context switches: 157
- Involuntary context switches: 745

## Command Failures

- `perf record`: exit 255; blocked by `perf_event_paranoid=4`; non-blocking because blockage transcript was captured.
- `perf stat`: exit 255; blocked by `perf_event_paranoid=4`; non-blocking because blockage transcript was captured.
- A local metadata inspection `rg` command used an unescaped backtick in its pattern and printed `zsh:1: command not found: release-perf`; it did not alter artifacts or source files.
