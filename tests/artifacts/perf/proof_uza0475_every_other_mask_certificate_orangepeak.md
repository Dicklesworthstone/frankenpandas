# br-frankenpandas-uza04.75 proof - every-other mask certificate

## Target

- Bead: `br-frankenpandas-uza04.75`
- Lever: recognize exact even/odd every-other boolean masks with a safe repeated-octet certificate path before the general affine builder.
- Scope: `crates/fp-frame/src/lib.rs`
- Baseline head: `8ab7f7b8`

## Baseline and profile

- Build: `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0475-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- RCH worker: `vmi1227854`; release-perf build passed.
- Baseline hyperfine: `filter_bool 100000 1000` = `96.3 ms +/- 6.2 ms`, 10 runs.
- Paired baseline before the candidate: `93.2 ms +/- 5.6 ms`, 12 runs.
- Baseline profile: `perf_profile filter_bool 100000 20000` = `0.050 ms/iter`, `sink=1000000000`.
- Baseline residual: `<fp_frame::DataFrame>::loc_bool` envelope at `92.35%` self; annotation showed the affine-mask certificate byte walk dominated the benchmark mask path.

## Implementation

- Added `boolean_mask_every_other_affine_certificate` before the existing general certificate builder.
- The helper validates exact repeated `[true, false, ...]` and `[false, true, ...]` octets, including tails, and returns the same arithmetic selection witnesses:
  - even positions: `start=0, step=2, len=ceil(mask.len()/2)`
  - odd positions: `start=1, step=2, len=floor(mask.len()/2)`
- All other masks fall through to the existing `AffineSelectionBuilder` path.
- Tiny masks with fewer than two selected positions fall through so existing singleton step handling remains unchanged.

## Isomorphism

- Row order: unchanged; the returned certificate enumerates the same selected positions in ascending order.
- Index labels: unchanged; downstream `take_rows_by_affine_certificate_unchecked` is unchanged and receives the same logical `(start, step, len)` for every matched pattern.
- Column order and names: unchanged; output frame construction is unchanged.
- Dtypes, validity, nulls, NaNs, and f64 bits: unchanged; no column data path changed.
- Fallback behavior: any non-even/odd repeated-octet mask still uses the old general scanner.
- Tie-breaking: not applicable to boolean filtering.
- RNG: not used.

## Golden proof

Baseline SHA-256:

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/uza0475_base_golden_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/uza0475_base_golden_filter_bool_100000.txt
```

After SHA-256:

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/uza0475_after_golden_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/uza0475_after_golden_filter_bool_100000.txt
```

Hash-only diff: empty.

## Performance

Build after:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0475-after RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Paired hyperfine:

```text
baseline: 93.2 ms +/- 5.6 ms
after:    47.0 ms +/- 3.4 ms
summary:  after ran 1.98x +/- 0.19 faster
```

Reversed paired hyperfine:

```text
after:    50.7 ms +/- 4.7 ms
baseline: 94.5 ms +/- 5.2 ms
summary:  after ran 1.86x +/- 0.20 faster
```

Score: Impact 4 x Confidence 4 / Effort 2 = 8.0, keep.

## Validation

Passed:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0475-check rch exec -- cargo test -p fp-frame boolean_mask_affine_certificate_recognizes_every_other_octets --lib -- --nocapture
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0475-check rch exec -- cargo check -p fp-frame --lib
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0475-clippy rch exec -- cargo clippy -p fp-frame --lib -- -D warnings
git diff --check -- crates/fp-frame/src/lib.rs
```

Caveats:

```text
cargo fmt -p fp-frame -- --check
```

Still fails on broad pre-existing formatting drift in `crates/fp-frame/examples/*.rs`, outside the touched file.

```text
timeout 180 ubs crates/fp-frame/src/lib.rs
```

Timed out after printing the UBS banner and `Scanning rust...`; no findings were emitted.

## After profile

- `perf_profile filter_bool 100000 20000`: `0.009 ms/iter`, `sink=1000000000`.
- The mask-certificate scan is no longer the residual.
- New residual: `<fp_frame::DataFrame>::new_with_axes` at `75.16%` self, dominated by `HashMap<&str, ()>::insert` / SipHash during column-order normalization.
- Follow-up route: internal prevalidated output-frame constructor or normalization bypass for affine boolean filter outputs, preserving duplicate-column rejection and all public constructor behavior.

## Artifacts

- `tests/artifacts/perf/uza0475_base_*`
- `tests/artifacts/perf/uza0475_after_*`
- `tests/artifacts/perf/uza0475_pair_*`
- `tests/artifacts/perf/uza0475_test_every_other_certificate.txt`
- `tests/artifacts/perf/uza0475_check_fp_frame_lib.txt`
- `tests/artifacts/perf/uza0475_clippy_fp_frame_lib.txt`
- `tests/artifacts/perf/uza0475_fmt_fp_frame_check.txt`
- `tests/artifacts/perf/uza0475_ubs_fp_frame.txt`
