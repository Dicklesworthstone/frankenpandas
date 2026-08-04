# br-frankenpandas-uza04.122 - Index::new unit-range canonicalization rejection

Date: 2026-06-14
Agent: LavenderStone

## Target

Fresh target was `series_add_align 100000 100` from the active alignment
hotspot. Prior profile/routing artifacts show the stack spending time under
alignment semantic witness construction, including duplicate detection,
semantic index identity, and label fingerprinting. `fp-frame` already has a
fast semantic fingerprint branch for `Index::int64_unit_range_labels()`, while
public `Index::new(Vec<IndexLabel>)` kept exact Int64 unit ranges materialized.

## Lever Tested

Not retained. The candidate recognized exact public `Vec<IndexLabel>` sequences
of `Int64(start), Int64(start + 1), ...`, replaced the `IndexLabels` backing
with the existing lazy unit-range representation, and seeded duplicate/sort
caches. Non-unit, mixed, and duplicate labels stayed on the old path in the
candidate tests.

## Build And RCH

The baseline and candidate benchmark builds were invoked through RCH with
separate target dirs:

- Baseline: `CARGO_TARGET_DIR=.rch-target-lavender-index-range-base RUSTFLAGS="-C force-frame-pointers=yes" rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- Candidate: `CARGO_TARGET_DIR=.rch-target-lavender-index-range-after RUSTFLAGS="-C force-frame-pointers=yes" rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`

RCH failed open locally for both builds with no admissible worker. Build logs:

- `tests/artifacts/perf/lavender_index_new_unit_range_base_build_perf_profile.txt`
- `tests/artifacts/perf/lavender_index_new_unit_range_after_build_perf_profile.txt`

## Behavior Proof

Golden commands:

- `.rch-target-lavender-index-range-base/release-perf/examples/perf_profile golden series_add_align 1000`
- `.rch-target-lavender-index-range-base/release-perf/examples/perf_profile golden series_add_align 100000`
- `.rch-target-lavender-index-range-after/release-perf/examples/perf_profile golden series_add_align 1000`
- `.rch-target-lavender-index-range-after/release-perf/examples/perf_profile golden series_add_align 100000`

Before and after goldens matched byte-for-byte with `cmp -s`.

Hashes:

- `series_add_align 1000`: `dbc710bc7225d1b7ad858689e228a41a07eb8415dc43af2916db94020aa47d4f`
- `series_add_align 100000`: `2b716d47c784429314fd64c9aa24ae9ca8fa4569e928c317fc8c2e1258c4b5c2`

Isomorphism notes:

- Ordering: unchanged; all output rows compare byte-identical.
- Tie-breaking: unchanged; no candidate path changed alignment emission order.
- Floating point: unchanged; output golden bytes match exactly.
- RNG: not used by this benchmark path.
- Null/NaN: unchanged by byte-identical golden output.

## Bench Gate

Baseline-only hyperfine:

- Baseline `series_add_align 100000 100`: `68.3 ms +/- 2.6 ms`

Paired forward:

- Baseline: `68.8 ms +/- 3.3 ms`
- Candidate: `1.128 s +/- 0.034 s`
- Baseline was `16.40x` faster.

Paired reversed:

- Candidate: `1.229 s +/- 0.168 s`
- Baseline: `69.2 ms +/- 2.1 ms`
- Baseline was `17.76x` faster.

Artifacts:

- `tests/artifacts/perf/lavender_index_new_unit_range_base_hyperfine_series_add_align_100000x100.txt`
- `tests/artifacts/perf/lavender_index_new_unit_range_base_hyperfine_series_add_align_100000x100.json`
- `tests/artifacts/perf/lavender_index_new_unit_range_pair_forward.txt`
- `tests/artifacts/perf/lavender_index_new_unit_range_pair_forward.json`
- `tests/artifacts/perf/lavender_index_new_unit_range_pair_reversed.txt`
- `tests/artifacts/perf/lavender_index_new_unit_range_pair_reversed.json`

## Verdict

Rejected. Score is below the keep threshold because the candidate is a large
regression despite exact behavior preservation.

Diagnosis: this workload starts with an already-materialized public
`Vec<IndexLabel>`. Naively replacing it with a lazy range discards that backing,
adds a constructor scan, and later rematerializes labels for paths that still
need the slice. The semantic-fingerprint fast path does not compensate for the
lost materialized backing here.

Do not retry naive public-constructor unit-range canonicalization. The deeper
primitive should preserve materialized labels while carrying a sidecar
unit-range semantic witness, or move the alignment witness path to consume typed
Int64/range metadata without forcing `Index::new` to throw away its input.

Runtime source hunk removed before closeout; this commit is evidence only.
