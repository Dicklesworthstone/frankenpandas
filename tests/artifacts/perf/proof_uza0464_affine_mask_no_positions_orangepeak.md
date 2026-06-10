# br-frankenpandas-uza04.64 proof: affine filter mask without positions tape

Agent: OrangePeak
Date: 2026-06-10
Bead: br-frankenpandas-uza04.64

## Profile-backed target

Fresh crate-scoped baseline built with:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0464-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Baseline binary:

```text
/data/projects/.scratch/cargo-target-orangepeak-uza0464-base/release-perf/examples/perf_profile
```

Baseline profile for `filter_bool 100000 1000` kept `DataFrame::loc_bool` hot at 77.36%. Inside it the old positions path showed `boolean_mask_positions` plus `Vec<usize>` pushes:

```text
push_mut<usize>                         3.85%
push<usize>                             3.29%
boolean_mask_positions                  3.22%
```

That matches the bead target after `.63`: mask-to-position construction cost after the affine certificate route.

## One lever

The single lever is a certified affine boolean-mask fast path that does not materialize the selected `Vec<usize>`.

`boolean_mask_affine_certificate` scans the mask directly into an `AffineSelectionBuilder`, exits as soon as the true positions stop being an arithmetic progression, and returns a witness `(start, step, len)`.

`DataFrame::loc_bool` now tries `take_rows_by_affine_certificate_unchecked` before the old `boolean_mask_positions` path. The fast path is accepted only when:

- The witness is non-empty and in bounds.
- The frame has no row multiindex.
- The index projection can be produced from the same arithmetic positions.
- Every column accepts the hidden descriptor-only affine gather `Column::take_affine_positions_without_materialized_positions`.

If any condition fails, execution falls back to the previous materialized positions path.

## Behavior isomorphism

- Selected row sequence is identical: the certificate represents exactly `start + i * step` for `i in 0..len`, the same positions the previous `boolean_mask_positions` scan would have pushed for affine masks.
- Row ordering is preserved because both paths emit selected positions in increasing mask-scan order.
- Index labels are selected at the same arithmetic positions. Unit-range `Int64` indexes use the equivalent range constructor for step 1; strided and cached-label cases push labels in the same order and with checked overflow. Generic labels clone from `self.index.labels()[pos]`, matching the old gather semantics.
- Column names, column order, index name, and column multiindex are passed through unchanged via `Self::new_with_axes`.
- Column dtype, validity, null/NaN semantics, and f64 bit patterns are unchanged on the accepted path because each column must prove descriptor-only affine support through the existing all-valid Float64 lazy strided representation. Unsupported columns fall back to `take_positions`.
- Panic and error behavior is preserved for valid callers: out-of-bounds or overflowed witnesses are rejected to fallback/None instead of being used. `loc_bool` still performs the original mask-length validation before attempting the fast path.
- Tie-breaking is not applicable: boolean filtering preserves row order and performs no comparisons.
- RNG is not used.

## Golden-output proof

```text
filter_bool 1000 baseline sha256:
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c

filter_bool 1000 after sha256:
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c

filter_bool 100000 baseline sha256:
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea

filter_bool 100000 after sha256:
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea

golden_cmp_1000=0
golden_cmp_100000=0
```

Both `sha256sum -c` verification files passed.

## Benchmarks

Direct smoke timing:

```text
baseline: 0.168 ms/iter, sink=50000000
after:    0.156 ms/iter, sink=50000000
```

Paired hyperfine, 20 runs, warmup 3, `filter_bool 100000 1000`:

```text
baseline: 175.3 ms +/- 7.7 ms
after:    158.4 ms +/- 11.3 ms
summary:  after ran 1.11 +/- 0.09 times faster than base
```

## After-profile and shifted hotspot

After-profile still places `DataFrame::loc_bool` at 74.38%, but the old `boolean_mask_positions` and `Vec<usize>` push frames are gone from the visible hot list. The shifted residual is now affine index-label projection:

```text
push<i64>                                      6.73%
spec_next<usize>                               6.11%
take_rows_by_affine_certificate_unchecked      5.69%
deref<i64>                                     5.07%
boolean_mask_affine_certificate                4.76%
```

Follow-up bead filed: `br-frankenpandas-uza04.74` targets affine filter index-label materialization.

## Validation

Focused behavior tests passed:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0464-check rch exec -- cargo test -p fp-frame boolean_mask_positions_tracks_affine_certificate --lib
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0464-check rch exec -- cargo test -p fp-frame dataframe_loc_bool_selects_true_rows --lib
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0464-check rch exec -- cargo test -p fp-columnar take_positions_with_affine_certificate_uses_lazy_strided_float64 --lib
```

Crate-scoped checks:

```text
cargo fmt -p fp-frame -p fp-columnar -- --check
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0464-check rch exec -- cargo check -p fp-frame -p fp-columnar --all-targets
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0464-check rch exec -- cargo clippy -p fp-frame --lib -- -D warnings
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0464-check rch exec -- cargo clippy -p fp-columnar --lib -- -D warnings
```

These passed. The broad all-target clippy attempt is still blocked by unrelated example lints in `crates/fp-frame/examples/corr_parity_dump.rs`.

UBS:

```text
ubs crates/fp-columnar/src/lib.rs
```

Completed with no critical issues and only existing broad warning inventory.

```text
ubs crates/fp-frame/src/lib.rs
ubs --skip-rust=1 crates/fp-frame/src/lib.rs
```

Both `fp-frame` UBS attempts timed out at 240s without emitting findings. The first stalled in Rust category 1's AST inventory pass; the second also timed out after skipping that category. The proof/progress/tracker files report "no recognizable languages".

## Score

Impact 3 x Confidence 4 / Effort 2 = 6.00.

Verdict: keep. The candidate clears the Score >= 2.0 gate with unchanged golden output and a profile-confirmed removal of the targeted positions-vector hotspot.
