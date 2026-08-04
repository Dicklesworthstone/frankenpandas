# br-frankenpandas-uza04.123 - sorted-unique union semantic identity rejection

Date: 2026-06-14
Agent: LavenderStone

## Target

Profile-backed target: `series_add_align 100000 100`. Prior stack samples for
this scenario include `record_alignment_semantic_witness`,
`semantic_index_identity`, duplicate detection, and index-label fingerprinting.
After `br-frankenpandas-uza04.122` rejected public `Index::new` range
canonicalization, this pass tested a narrower semantic-witness cache lever that
does not enable the Int64 unit-range arithmetic fast path.

## Lever Tested

Not retained. Candidate changed `semantic_sorted_unique_union_output_fingerprint`
to return a full `SemanticIndexIdentity` for sorted-unique union outputs. The
existing preconditions already require left and right indexes to be duplicate
free and sorted, so the candidate set output `has_duplicates=false` directly and
avoided calling `output_index.has_duplicates()` on cached output-fingerprint
hits.

The alignment plan, output labels, column arithmetic, ordering, and cache key
were otherwise unchanged.

## Build And RCH

Baseline and candidate benchmark builds were invoked through RCH with separate
target dirs:

- Baseline: `CARGO_TARGET_DIR=.rch-target-lavender-sorted-union-base RUSTFLAGS="-C force-frame-pointers=yes" rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- Candidate: `CARGO_TARGET_DIR=.rch-target-lavender-sorted-union-after RUSTFLAGS="-C force-frame-pointers=yes" rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`

RCH failed open locally for both builds because no worker was admissible. Logs:

- `tests/artifacts/perf/lavender_sorted_union_identity_base_build_perf_profile.txt`
- `tests/artifacts/perf/lavender_sorted_union_identity_after_build_perf_profile.txt`

## Behavior Proof

Before and after goldens matched byte-for-byte with `cmp -s`.

Hashes:

- `series_add_align 1000`: `dbc710bc7225d1b7ad858689e228a41a07eb8415dc43af2916db94020aa47d4f`
- `series_add_align 100000`: `2b716d47c784429314fd64c9aa24ae9ca8fa4569e928c317fc8c2e1258c4b5c2`

Isomorphism notes:

- Ordering/tie-breaking: unchanged; output golden bytes match exactly.
- Floating point: unchanged; candidate did not alter arithmetic and golden bytes match.
- RNG: not used.
- Null/NaN: unchanged by byte-identical output.
- Sorted-unique proof: focused candidate tests passed for cache preservation,
  mismatch rejection, eviction, and thread-safety before the source hunk was
  removed.

## Bench Gate

Baseline-only hyperfine:

- Baseline `series_add_align 100000 100`: `69.2 ms +/- 4.3 ms`

Paired forward:

- Baseline: `69.6 ms +/- 3.4 ms`
- Candidate: `71.3 ms +/- 1.8 ms`
- Baseline was `1.02x +/- 0.06` faster.

Paired reversed:

- Candidate: `73.8 ms +/- 3.1 ms`
- Baseline: `72.3 ms +/- 4.0 ms`
- Baseline was `1.02x +/- 0.07` faster.

Artifacts:

- `tests/artifacts/perf/lavender_sorted_union_identity_base_hyperfine_series_add_align_100000x100.txt`
- `tests/artifacts/perf/lavender_sorted_union_identity_base_hyperfine_series_add_align_100000x100.json`
- `tests/artifacts/perf/lavender_sorted_union_identity_pair_forward.txt`
- `tests/artifacts/perf/lavender_sorted_union_identity_pair_forward.json`
- `tests/artifacts/perf/lavender_sorted_union_identity_pair_reversed.txt`
- `tests/artifacts/perf/lavender_sorted_union_identity_pair_reversed.json`

## Verdict

Rejected. Candidate preserved behavior but did not produce a measurable win and
was slightly slower in both pair orders, below Score>=2.0.

Diagnosis: the remaining repeated cost in `series_add_align` is not dominated
by the final cached-output `has_duplicates()` call, or the extra identity
construction/branching offsets it. Do not retry this semantic identity
micro-family. The next profile-backed attack should move deeper into the
alignment/data movement path: either avoid building the per-iteration
`union_labels`/position vectors for adjacent sorted Int64 ranges without
activating the regressed range column kernel, or attack a different measured
hotspot.

Runtime source hunk removed before closeout; this commit is evidence only.
