# br-frankenpandas-uza04.86 Pass 1 Baseline/Profile Witness

## Scope

- Bead: `br-frankenpandas-uza04.86`
- Owner: OrangePeak
- Target: producer-carried Bool affine mask certificates for first-use `filter_bool`.
- Current HEAD: `44cb282e`
- Baseline binary:
  `/data/projects/.scratch/cargo-target-orangepeak-uza0486-base/release-perf/examples/perf_profile`

## Build

```bash
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0486-base \
RUSTFLAGS='-C force-frame-pointers=yes' \
rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

- Result: passed through RCH on `vmi1227854`.
- Build log: `tests/artifacts/perf/uza0486_base_build_perf_profile.txt`.

## Golden Outputs

```bash
/data/projects/.scratch/cargo-target-orangepeak-uza0486-base/release-perf/examples/perf_profile \
  golden filter_bool 1000 \
  > tests/artifacts/perf/uza0486_orangepeak_base_golden_filter_bool_1000.txt

/data/projects/.scratch/cargo-target-orangepeak-uza0486-base/release-perf/examples/perf_profile \
  golden filter_bool 100000 \
  > tests/artifacts/perf/uza0486_orangepeak_base_golden_filter_bool_100000.txt

sha256sum -c tests/artifacts/perf/uza0486_orangepeak_base_golden_sha256.txt
```

Verified hashes:

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/uza0486_orangepeak_base_golden_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/uza0486_orangepeak_base_golden_filter_bool_100000.txt
```

`sha256sum -c` passed for both files.

## Baseline Timings

Commands:

```bash
rch exec -- hyperfine --warmup 3 --runs 10 \
  --export-json tests/artifacts/perf/uza0486_orangepeak_base_hyperfine_filter_bool_100000x1000.json \
  '/data/projects/.scratch/cargo-target-orangepeak-uza0486-base/release-perf/examples/perf_profile filter_bool 100000 1000'

rch exec -- hyperfine --warmup 3 --runs 10 \
  --export-json tests/artifacts/perf/uza0486_orangepeak_base_hyperfine_filter_bool_100000x20000.json \
  '/data/projects/.scratch/cargo-target-orangepeak-uza0486-base/release-perf/examples/perf_profile filter_bool 100000 20000'
```

RCH warns for non-compilation commands, but the same RCH-built binary was used.

Results:

| Workload | Mean | Stddev | Range |
| --- | ---: | ---: | ---: |
| `filter_bool 100000 1000` | `19.6 ms` | `1.7 ms` | `17.3..22.3 ms` |
| `filter_bool 100000 20000` | `65.1 ms` | `3.8 ms` | `61.1..72.6 ms` |

## Profile Attempt

```bash
timeout 90s perf record -F 997 -g \
  -o tests/artifacts/perf/uza0486_orangepeak_base_perf_filter_bool_100000x20000.data \
  -- /data/projects/.scratch/cargo-target-orangepeak-uza0486-base/release-perf/examples/perf_profile \
  filter_bool 100000 20000
```

Result: blocked by host policy. `perf_event_paranoid` is `4`, so unprivileged
performance monitoring is unavailable. The zero-byte `.data` file and stderr
capture are retained as failed-profile evidence:

- `tests/artifacts/perf/uza0486_orangepeak_base_perf_filter_bool_100000x20000.data`
- `tests/artifacts/perf/uza0486_orangepeak_base_perf_record_filter_bool_100000x20000.txt`

## Routing Evidence

The current `filter_bool` benchmark path builds an immutable all-valid Bool
`Series` and calls `DataFrame::filter_rows`. For identical non-duplicate indexes,
`filter_rows` obtains `mask.column().as_bool_slice()` and then asks
`mask.column().bool_affine_selection_witness()`. That witness currently scans
the bool backing the first time it is requested. `.85` already proved repeated
use benefits once the witness is cached; `.86` should remove the first-use scan
by making the producer carry an exact affine witness or affine Bool
representation.

Behavior proof anchor for every candidate:

- row order and index labels must remain the affine selected sequence;
- column order, names, dtypes, validity, null/NaN and f64 bit payloads must be
  identical to the baseline goldens;
- raw `loc_bool(&[bool])` / `iloc_bool(&[bool])` must remain conservative and
  mutation-visible;
- nullable, non-Bool, misaligned, and duplicate-index masks must keep existing
  fallback behavior.
