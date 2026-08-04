# br-frankenpandas-uza04.83 rejection proof - pairwise period-2 verifier

Timestamp: 2026-06-12T22:31:00Z
Owner: OrangePeak

## Candidate

Replace the every-other mask verifier's repeated 8-bool pattern check with a
direct pairwise period-2 proof:

- accept `[true, false, true, false, ...]` as `{ start: 0, step: 2 }`
- accept `[false, true, false, true, ...]` as `{ start: 1, step: 2 }`
- scan the full raw `[bool]` slice so caller-visible mutation semantics remain
  conservative
- keep the general affine fallback unchanged

The intent was to reduce the profile-visible period-2 verifier cost without
repeating the rejected 64-byte block verifier.

## Behavior Proof

Focused test passed remotely:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0483-pass2-test
rch exec -- cargo test -p fp-frame boolean_mask_affine_certificate_recognizes_every_other_octets --lib
```

Golden SHA stayed unchanged:

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  filter_bool 1000
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  filter_bool 100000
```

`cmp` matched baseline files for both sizes.

Isomorphism:

- Ordering and selected positions are identical for both alternating masks.
- Index/column names/order/multiindex are untouched.
- Dtypes, validity, nulls, NaNs, f64 bits, and scalar rendering are untouched.
- Non-alternating masks still fall through to the existing general affine scan.
- Tie-breaking and RNG are not part of boolean row filtering.

## Benchmark

Release-perf build passed remotely on `vmi1227854`:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0483-pass2
RUSTFLAGS='-C force-frame-pointers=yes'
rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Paired hyperfine (`filter_bool 100000 1000`, 20 runs):

```text
base:      23.317 ms +/- 2.437 ms
candidate: 45.718 ms +/- 2.246 ms
baseline ran 1.96x +/- 0.23 faster
```

## Verdict

Rejected. Score: Impact 0 x Confidence 5 / Effort 1 = 0.0. The production hunk
was removed.

Next route: do not repeat pairwise/block-size raw-slice verifier rewrites. A
real next swing needs a different representation boundary, such as a safe
immutable mask proof object or typed boolean mask column path that can carry a
certificate without rescanning a mutable borrowed `[bool]` slice.
