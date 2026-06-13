# br-frankenpandas-uza04.86 Rejection Proof

## Candidate

Single lever tested:
seed all-valid Bool affine-selection witnesses at the producer boundary for the
profiled every-other `filter_bool` mask. The candidate added a checked
constructor that builds the dense Bool mask and preloads the immutable witness
`{ start: 0, step: 2, len: ceil(n / 2) }`, then changed only the benchmark mask
producer to use it.

The lever was tested in an isolated detached after worktree from `44cb282e`.
No candidate source hunks are retained in the main working tree after the
rejection.

## Build Evidence

Baseline:

```bash
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0486-base \
RUSTFLAGS='-C force-frame-pointers=yes' \
rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

After:

```bash
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0486-after \
RUSTFLAGS='-C force-frame-pointers=yes' \
rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Both builds ran through RCH on `vmi1227854`. The candidate focused test also
passed remotely:

```bash
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0486-test \
rch exec -- cargo test -p fp-columnar \
  bool_affine_selection_mask_constructor_seeds_witness_uza0486 --lib
```

## Golden Proof

Baseline SHA:

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/uza0486_base_golden_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/uza0486_base_golden_filter_bool_100000.txt
```

After SHA:

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/uza0486_after_golden_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/uza0486_after_golden_filter_bool_100000.txt
```

`cmp` verification returned 0 for both output sizes:

```text
cmp1000=0
cmp100000=0
```

Isomorphism obligations:

- Ordering: unchanged; selected positions are the same affine sequence.
- Tie-breaking: unchanged; filtering does not introduce a comparator or tie
  rule.
- Floating point: unchanged; payload columns are gathered without recomputing
  f64 values.
- RNG: unchanged; the workload has no RNG.
- Fallbacks: raw `loc_bool(&[bool])`, raw `iloc_bool(&[bool])`, nullable masks,
  non-Bool masks, misaligned masks, and duplicate-index paths were not changed.

## Timings

Forward paired gate:

| Workload | Baseline | After | Verdict |
| --- | ---: | ---: | --- |
| `filter_bool 100000 1000` | `18.6 ms +/- 1.5` | `19.7 ms +/- 2.2` | baseline `1.06x +/- 0.14` faster |
| `filter_bool 100000 20000` | `63.4 ms +/- 2.5` | `65.7 ms +/- 2.9` | baseline `1.04x +/- 0.06` faster |

Reversed paired gate:

| Workload | After | Baseline | Verdict |
| --- | ---: | ---: | --- |
| `filter_bool 100000 1000` | `18.6 ms +/- 1.7` | `19.1 ms +/- 2.5` | after `1.02x +/- 0.16` faster |
| `filter_bool 100000 20000` | `68.0 ms +/- 4.9` | `73.5 ms +/- 7.4` | after `1.08x +/- 0.13` faster |

The reversed long run is noise and below the keep threshold. The forward run
shows a regression. The lever does not clear Score >= 2.0.

## Profile Limitation

The kernel profile attempt was blocked by host policy:
`perf_event_paranoid=4`. The failed profile data and stderr are retained under:

- `tests/artifacts/perf/uza0486_orangepeak_base_perf_filter_bool_100000x20000.data`
- `tests/artifacts/perf/uza0486_orangepeak_base_perf_record_filter_bool_100000x20000.txt`

Routing therefore used the refreshed baseline timings plus the `.85` proof that
warm typed Bool witness reuse already cleared the repeated long gate.

## Decision

Rejected. The candidate preserved behavior but failed the paired/reversed
performance gate:

```text
Impact 0 x Confidence 3 / Effort 1 = 0.0
```

This is the third failed/rejected Bool certificate micro-family after the prior
raw verifier and block verifier attempts. The next optimization pass should
leave first-use Bool witness micro-tuning and attack a deeper primitive:
either eliminate dense mask materialization with an affine/selection-vector
representation, or switch to the next ready profile-backed bead with an
algorithmic target.
