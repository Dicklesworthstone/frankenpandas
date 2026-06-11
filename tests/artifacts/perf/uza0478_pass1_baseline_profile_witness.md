# br-frankenpandas-uza04.78 Pass 1 Baseline/Profile Witness

Timestamp: 2026-06-11T14:08:10Z
Worktree: `/data/projects/.scratch/frankenpandas-codex-opt-20260611`
Head: `7b3754255b4e27b3ce3b9ef3cdf257a700212bd9`
Target: `filter_bool 100000 1000` current `.78` baseline/profile only.

## Bead State

`br show br-frankenpandas-uza04.78 --json` reported status `in_progress`,
assignee `Codex`, labels `filter-bool`, `fp-frame`, `no-gaps`, `perf`.

## Build

Command:

```bash
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-codex-uza0478-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

RCH result: fail-open local. Transcript starts with
`[RCH] local (no admissible workers: insufficient_slots=1,hard_preflight=1)`.
Build passed in `2m 10s`.

Artifact: `tests/artifacts/perf/uza0478_base_build_perf_profile.txt`

## Golden Outputs

Commands:

```bash
/data/projects/.scratch/cargo-target-codex-uza0478-base/release-perf/examples/perf_profile golden filter_bool 1000 > tests/artifacts/perf/uza0478_base_golden_filter_bool_1000.txt
/data/projects/.scratch/cargo-target-codex-uza0478-base/release-perf/examples/perf_profile golden filter_bool 100000 > tests/artifacts/perf/uza0478_base_golden_filter_bool_100000.txt
sha256sum tests/artifacts/perf/uza0478_base_golden_filter_bool_1000.txt tests/artifacts/perf/uza0478_base_golden_filter_bool_100000.txt > tests/artifacts/perf/uza0478_base_golden_sha256.txt
sha256sum -c tests/artifacts/perf/uza0478_base_golden_sha256.txt
```

SHA256:

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/uza0478_base_golden_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/uza0478_base_golden_filter_bool_100000.txt
```

Verification passed:

```text
tests/artifacts/perf/uza0478_base_golden_filter_bool_1000.txt: OK
tests/artifacts/perf/uza0478_base_golden_filter_bool_100000.txt: OK
```

Artifacts:

- `tests/artifacts/perf/uza0478_base_golden_filter_bool_1000.txt`
- `tests/artifacts/perf/uza0478_base_golden_filter_bool_100000.txt`
- `tests/artifacts/perf/uza0478_base_golden_sha256.txt`
- `tests/artifacts/perf/uza0478_base_golden_verify.txt`

## Baseline Timing

RCH-wrapped command:

```bash
rch exec -- hyperfine --warmup 3 --runs 10 --export-json tests/artifacts/perf/uza0478_base_rch_hyperfine_filter_bool_100000x1000.json '/data/projects/.scratch/cargo-target-codex-uza0478-base/release-perf/examples/perf_profile filter_bool 100000 1000'
```

RCH note: `rch exec` emitted its non-compilation-command warning for
`hyperfine`; the benchmark ran locally against the built binary.

Result:

```text
Time (mean +/- sigma): 49.1 ms +/- 4.3 ms
Range: 43.0 ms .. 57.1 ms
Runs: 10
Median from JSON: 47.9 ms
```

Direct corroborating command:

```bash
hyperfine --warmup 3 --runs 10 --export-json tests/artifacts/perf/uza0478_base_hyperfine_filter_bool_100000x1000.json '/data/projects/.scratch/cargo-target-codex-uza0478-base/release-perf/examples/perf_profile filter_bool 100000 1000'
```

Direct corroborating result: `49.2 ms +/- 3.6 ms`, range `43.3 ms .. 56.9 ms`.

Artifacts:

- `tests/artifacts/perf/uza0478_base_rch_hyperfine_filter_bool_100000x1000.txt`
- `tests/artifacts/perf/uza0478_base_rch_hyperfine_filter_bool_100000x1000.json`
- `tests/artifacts/perf/uza0478_base_hyperfine_filter_bool_100000x1000.txt`
- `tests/artifacts/perf/uza0478_base_hyperfine_filter_bool_100000x1000.json`

## CPU Profile

Command:

```bash
perf record -F 999 -g -o tests/artifacts/perf/uza0478_base_perf_filter_bool_100000x20000.data -- /data/projects/.scratch/cargo-target-codex-uza0478-base/release-perf/examples/perf_profile filter_bool 100000 20000
```

Perf was usable. Kernel symbols were restricted by `/proc/kallsyms` permission,
but user-space symbols were recorded.

Harness result:

```text
perf_profile: scenario=filter_bool n=100000 iters=20000
perf_profile: done 20000 iters in 0.145s (0.007 ms/iter), sink=1000000000
Captured 164 samples, 0 lost samples.
```

Report commands:

```bash
perf report --stdio --no-children -i tests/artifacts/perf/uza0478_base_perf_filter_bool_100000x20000.data > tests/artifacts/perf/uza0478_base_perf_report_filter_bool_100000x20000.txt
perf report --stdio --children -i tests/artifacts/perf/uza0478_base_perf_filter_bool_100000x20000.data > tests/artifacts/perf/uza0478_base_perf_report_children_filter_bool_100000x20000.txt
perf annotate --stdio -i tests/artifacts/perf/uza0478_base_perf_filter_bool_100000x20000.data --symbol '<fp_frame::DataFrame>::loc_bool' > tests/artifacts/perf/uza0478_base_perf_annotate_loc_bool_filter_bool_100000x20000.txt
```

Top rows:

| Row | Evidence |
| --- | --- |
| `<fp_frame::DataFrame>::loc_bool` | `46.70%` self in no-children report; `71.27%` children under `perf_profile::main` in children report. |
| `<fp_frame::DataFrame>::new_with_axes` | `9.57%` children under `loc_bool`. |
| `BTreeMap<String, Column>::insert` | `8.84%` children under `loc_bool`. |
| `__memmove_avx_unaligned_erms` | `2.76%` children under `loc_bool`; also visible in frame build setup. |
| `__memcmp_avx2_movbe` | `2.71%` children under `loc_bool`. |
| `<fp_columnar::Column>::take_affine_all_valid_float64_positions` | `1.39%` children under `loc_bool`. |

`loc_bool` annotation concentrates local samples in the every-other mask
verifier loop:

```text
17.56%  test   %rsi,%rsi
7.37%   add    $0x8,%rsi
16.15%  cmp    %rdi,(%r8)
38.33%  lea    0x8(%r8),%r8
14.81%  je     29f2e0
```

Artifacts:

- `tests/artifacts/perf/uza0478_base_perf_filter_bool_100000x20000.data`
- `tests/artifacts/perf/uza0478_base_perf_record_filter_bool_100000x20000.txt`
- `tests/artifacts/perf/uza0478_base_perf_report_filter_bool_100000x20000.txt`
- `tests/artifacts/perf/uza0478_base_perf_report_children_filter_bool_100000x20000.txt`
- `tests/artifacts/perf/uza0478_base_perf_annotate_loc_bool_filter_bool_100000x20000.txt`

## Pass 1 Verdict

Baseline/profile evidence is captured. `.78` remains profile-backed on current
head. The profile still points at `DataFrame::loc_bool` and the every-other
mask verifier, so Pass 2 should select a deeper producer-carried or typed mask
witness primitive rather than another block-size verifier tweak.
