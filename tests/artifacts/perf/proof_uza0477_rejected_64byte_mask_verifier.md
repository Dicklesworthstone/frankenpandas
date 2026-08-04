# br-frankenpandas-uza04.77 Rejection Proof: 64-Byte Mask Verifier

Timestamp: 2026-06-11T04:35:00Z
Head: `167ff9bf` plus candidate source hunk during measurement

## Candidate

Replace the existing every-other mask verifier's 8-bool repeated-octet scan
with a 64-bool repeated-block verifier, keeping the same certificate shape:

- even mask: `{ start: 0, step: 2, len: ceil(mask.len() / 2) }`
- odd mask: `{ start: 1, step: 2, len: floor(mask.len() / 2) }`

The candidate was measured as one source lever and then removed because it did
not clear the performance gate.

## Behavior Proof

Golden SHA stayed unchanged:

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/uza0477_after_golden_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/uza0477_after_golden_filter_bool_100000.txt
```

`cmp` against the baseline golden files passed for both sizes.

Isomorphism obligations during the measured candidate:

- Row order preserved: same every-other certificate and fallback path.
- Index/column names/order preserved: no DataFrame construction changes.
- Dtype/validity/null/NaN behavior preserved: no column or scalar changes.
- Floating-point bits preserved: golden output bytes unchanged.
- Tie-breaking unchanged: N/A for boolean row selection.
- RNG unchanged: no RNG surface.
- Fallback behavior preserved: nonmatching masks still fall through to the
  existing general affine scan and materialized-position path.

Focused test:

```text
cargo test -p fp-frame boolean_mask_affine_certificate_recognizes_every_other_blocks --lib
1 passed
```

## Benchmark Result

Paired hyperfine, baseline first:

```text
baseline:  47.9 ms +/- 2.8 ms
candidate: 49.4 ms +/- 3.1 ms
baseline ran 1.03x +/- 0.09 faster
```

Paired hyperfine, candidate first:

```text
candidate: 50.2 ms +/- 3.1 ms
baseline:  47.8 ms +/- 2.8 ms
baseline ran 1.05x +/- 0.09 faster
```

## Verdict

Rejected. Score 0.0. The source hunk was removed and no production code was
retained.

Next route: do not repeat block-size verifier tweaks. Attack a deeper mask
primitive instead, such as a producer-carried immutable mask witness or a
typed/bitpacked boolean mask certificate that lets `loc_bool` accept known
generated masks without rescanning ordinary `[bool]` slices.
