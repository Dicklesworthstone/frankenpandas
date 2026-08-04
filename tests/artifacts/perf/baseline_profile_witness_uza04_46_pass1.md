# br-frankenpandas-uza04.62 Pass 1 Baseline/Profile Witness

Artifact note: this baseline was captured during the local loop originally
named `br-frankenpandas-uza04.46`. Current `origin/main` had already consumed
that Beads child id, so the integrated closeout uses
`br-frankenpandas-uza04.62` while retaining the `uza04_46` artifact filenames.

Timestamp: 2026-06-09T21:00:25Z
Head: 0650f4888beec52ebd8110539ca72055aadab1d3
Agent: OrangePeak

## Verdict

Target valid. The current-head `filter_bool 100000 1000` profile still spends
top-five child time in `bounded_arithmetic_progression_positions` under
`Column::take_positions` / `take_strided_all_valid_float64_positions`.

No implementation changes were made.

## Commands

```bash
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza04-46-pass1-worker \
  rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile \
  > tests/artifacts/perf/build_uza04_46_pass1_worker.txt 2>&1

hyperfine --warmup 3 --runs 10 \
  --export-json tests/artifacts/perf/hyperfine_uza04_46_pass1_filter_bool_100000_1000.json \
  '/data/projects/.scratch/cargo-target-orangepeak-uza04-46-pass1-worker/release-perf/examples/perf_profile filter_bool 100000 1000' \
  > tests/artifacts/perf/hyperfine_uza04_46_pass1_filter_bool_100000_1000.txt 2>&1

/data/projects/.scratch/cargo-target-orangepeak-uza04-46-pass1-worker/release-perf/examples/perf_profile \
  golden filter_bool 1000 \
  > tests/artifacts/perf/golden_uza04_46_pass1_filter_bool_1000.txt

/data/projects/.scratch/cargo-target-orangepeak-uza04-46-pass1-worker/release-perf/examples/perf_profile \
  golden filter_bool 100000 \
  > tests/artifacts/perf/golden_uza04_46_pass1_filter_bool_100000.txt

sha256sum \
  tests/artifacts/perf/golden_uza04_46_pass1_filter_bool_1000.txt \
  tests/artifacts/perf/golden_uza04_46_pass1_filter_bool_100000.txt \
  > tests/artifacts/perf/golden_uza04_46_pass1.sha256

sha256sum -c tests/artifacts/perf/golden_uza04_46_pass1.sha256 \
  > tests/artifacts/perf/golden_uza04_46_pass1.verify.txt

perf record -F 997 -g --call-graph dwarf \
  -o tests/artifacts/perf/perf_uza04_46_pass1_filter_bool_100000_1000.data \
  -- /data/projects/.scratch/cargo-target-orangepeak-uza04-46-pass1-worker/release-perf/examples/perf_profile \
  filter_bool 100000 1000 \
  > tests/artifacts/perf/perf_record_uza04_46_pass1_filter_bool_100000_1000.txt 2>&1

perf report --stdio --children --sort comm,dso,symbol \
  -i tests/artifacts/perf/perf_uza04_46_pass1_filter_bool_100000_1000.data \
  > tests/artifacts/perf/perf_report_uza04_46_pass1_filter_bool_100000_1000.txt 2>&1
```

## Build

- Artifact: `tests/artifacts/perf/build_uza04_46_pass1_worker.txt`
- RCH mode: local fail-open, `all workers failed preflight checks`
- Result: success, `release-perf` profile finished in 2m09s
- Binary: `/data/projects/.scratch/cargo-target-orangepeak-uza04-46-pass1-worker/release-perf/examples/perf_profile`

## Baseline

- Artifact text: `tests/artifacts/perf/hyperfine_uza04_46_pass1_filter_bool_100000_1000.txt`
- Artifact JSON: `tests/artifacts/perf/hyperfine_uza04_46_pass1_filter_bool_100000_1000.json`
- Workload: `filter_bool 100000 1000`
- Runs: 3 warmups, 10 measured
- Mean: 434.5 ms
- Stddev: 9.3 ms
- Median: 434.4146807 ms
- Min: 420.2396032 ms
- Max: 447.3973572 ms
- User: 406.4 ms
- System: 26.6 ms

## Goldens

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/golden_uza04_46_pass1_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/golden_uza04_46_pass1_filter_bool_100000.txt
```

Verification artifact: `tests/artifacts/perf/golden_uza04_46_pass1.verify.txt`

Both golden files verified OK.

## Profile

- Data: `tests/artifacts/perf/perf_uza04_46_pass1_filter_bool_100000_1000.data`
- Record log: `tests/artifacts/perf/perf_record_uza04_46_pass1_filter_bool_100000_1000.txt`
- Report: `tests/artifacts/perf/perf_report_uza04_46_pass1_filter_bool_100000_1000.txt`
- Samples: 565 cycles samples
- Runtime during record: 0.426s, 0.426 ms/iter, sink=50000000
- Policy note: perf succeeded; kernel maps were restricted, so kernel symbols may be unresolved. User-space symbols resolved.

Top profile rows by Children percent:

```text
58.46%  58.46%  perf_profile  perf_profile  [.] <fp_columnar::Column>::take_positions
58.22%   0.00%  perf_profile  perf_profile  [.] take_positions (inlined)
58.22%   0.00%  perf_profile  perf_profile  [.] take_strided_all_valid_float64_positions (inlined)
48.67%   0.00%  perf_profile  perf_profile  [.] bounded_arithmetic_progression_positions (inlined)
22.66%   0.00%  perf_profile  perf_profile  [.] loc_bool (inlined)
```

This profile confirms the bead route: repeated affine-position certificate work
inside per-column Float64 row filtering remains a valid next target.
