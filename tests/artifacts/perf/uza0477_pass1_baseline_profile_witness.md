# br-frankenpandas-uza04.77 Pass 1 Baseline/Profile Witness

Timestamp: 2026-06-11T04:17:00Z
Worktree: `/data/projects/.scratch/frankenpandas-codex-uza04-77-20260611`
Head: `167ff9bf` (`origin/main`)

## Target

`br-frankenpandas-uza04.77` targets the residual `filter_bool` mask witness
scan after the `.75` keep and `.76` constructor-bypass rejection.

## Build

Command:

```bash
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-silvercedar-uza0477-base \
RUSTFLAGS='-C force-frame-pointers=yes' \
rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

RCH worker: `vmi1227854`
Result: passed

## Golden SHA

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/uza0477_base_golden_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/uza0477_base_golden_filter_bool_100000.txt
```

`sha256sum -c tests/artifacts/perf/uza0477_base_golden_sha256.txt` passed.

## Baseline Timing

Command:

```bash
hyperfine --warmup 3 --runs 10 \
  '/data/projects/.scratch/cargo-target-silvercedar-uza0477-base/release-perf/examples/perf_profile filter_bool 100000 1000'
```

Result: `60.3 ms +/- 4.6 ms`, range `54.7..68.5 ms`.

## Profile

Command:

```bash
perf record -F 999 -g \
  -o tests/artifacts/perf/uza0477_base_perf_filter_bool_100000x20000.data \
  -- /data/projects/.scratch/cargo-target-silvercedar-uza0477-base/release-perf/examples/perf_profile filter_bool 100000 20000
```

Harness result: `0.009 ms/iter`, `sink=1000000000`, 270 samples, 0 lost samples.

Top profile rows:

- `<fp_frame::DataFrame>::loc_bool`: 46.46% self, 55.15% children.
- `__memmove_avx_unaligned_erms`: 20.87% children, 2.54% self.
- `BTreeMap<String, Column>::insert`: 4.02% self.

Annotation of `<fp_frame::DataFrame>::loc_bool` shows the hot steady-state
inside the every-other mask verifier. The compiler already lowers the current
8-bool repeated-octet loop into one unaligned 64-bit compare per 8 mask bytes;
local samples concentrate around the loop test/add/compare/load sequence.

## Verdict

The bead remains profile-backed. The next lever should target the residual
every-other mask verification loop without changing row selection semantics.
