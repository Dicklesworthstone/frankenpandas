# br-frankenpandas-uza04.65 compact descriptor rejection

Agent: OrangePeak
Date: 2026-06-10
Target: `perf_profile outer_join 500000`

## Profile-backed target

Pass 1 rebuilt the current `perf_profile` example crate-scoped through RCH
fail-open local execution:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0465-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Baseline evidence:

- Golden SHA for `outer_join 20000`:
  `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`.
- Hyperfine baseline `outer_join 500000 20`: `301.2 ms +/- 17.4 ms`.
- Direct confirmation `outer_join 500000 200`: `1.625 s`, `8.123 ms/iter`.
- Profile: `build_single_key_dense_i64_outer_merge_output` `83.99%` children
  / `23.69%` flat; CSR closure `26.16%` children / `19.65%` flat; tuple plan
  push path `14.00%` children; descriptor Arc materialization paths `8.55%`
  and `7.49%` children.

## Lever rejected

One safe-Rust lever was attempted in `crates/fp-join/src/lib.rs`: pre-count the
dense outer descriptor rows, emit `run_lens`, `left_run_valid`,
`left_run_positions`, and `right_segments` directly during the existing bucket
walk, and remove the temporary `(left_pos, right_start, run_len)` tuple plan plus
follow-up `plan.iter().map(...).collect()` passes.

This intentionally did not repeat:

- `.59` periodic CSR certification.
- `.60` periodic run-schedule certification.
- `.61` source-position promoted lanes.
- `.62` Arc-backed source provenance.

The candidate hunk was removed after the score gate rejected it. No production
source code is retained for this lever.

## Isomorphism proof while candidate was present

- Ordering: unchanged; descriptors were emitted at the exact sites where the
  baseline pushed plan tuples.
- Tie-breaking: unchanged; matched buckets kept left-major order and reused the
  same right CSR segment order.
- Null/NaN semantics: unchanged; the same side-only branches updated the same
  sparse invalid ranges, validity flags, and `NONE_POS` sentinel semantics.
- Dtype promotion: unchanged; side promotion gates and column constructors were
  untouched.
- Floating point: unchanged; no numeric cast site changed, and present promoted
  values still use Rust `i64 as f64`.
- RNG/hash behavior: unchanged; this path has no randomized iteration.

Baseline and candidate golden outputs matched byte-for-byte:

```text
base_sha=453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750
after_sha=453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750
golden_cmp=0
```

## Benchmark gate

Paired `outer_join 500000 20`:

- Baseline: `277.5 ms +/- 26.9 ms`.
- Candidate: `283.1 ms +/- 35.1 ms`.
- Ratio: baseline ran `1.02x +/- 0.16` faster.

Longer paired `outer_join 500000 200`:

- Baseline: `1.539 s +/- 0.023 s`.
- Candidate: `1.550 s +/- 0.051 s`.
- Ratio: baseline ran `1.01x +/- 0.04` faster.

Score: Impact `0` x Confidence `4` / Effort `2` = `0.0`; reject.

## Validation

Passed while the candidate was present:

```text
cargo fmt -p fp-join -- --check
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0465-after-test rch exec -- cargo test -p fp-join merge_outer_dense_int64_duplicates_matches_generic_validated_route --lib
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0465-after RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

After rejecting the candidate, `git diff -- crates/fp-join/src/lib.rs` was empty
and `cargo fmt -p fp-join -- --check` passed again.

## Next route

Do not repeat descriptor remapping (`.54` / `.65`), cursor-free CSR (`.56`),
periodic CSR (`.59`), or periodic run-schedule (`.60`) as a per-merge proof.
The next primitive should move the proof boundary: cache an exact dense-cycle
Int64 key witness with the column or first-use metadata, then let dense outer
join consume that compiled artifact to avoid per-merge CSR/plan reconstruction
for repeated joins while preserving fallback semantics for non-certified input.
