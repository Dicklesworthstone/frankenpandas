# br-frankenpandas-uza04.57 rejection proof - dense outer batched value tapes

## Baseline and profile

- Head: `2eab18bf`.
- Build: `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0457-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`.
- Golden baseline command: `perf_profile golden outer_join 20000`.
- Golden baseline SHA256: `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`.
- Baseline hyperfine: `outer_join 500000 20` = `377.7 ms +/- 14.7 ms`.
- Baseline profile: `build_single_key_dense_i64_outer_merge_output` = `90.93%` children; promoted right f64 tape closure = `14.49%` self; promoted left f64 run-values closure = `13.39%` self; `Column::from_f64_nullable_repeated_slices_shared` = `12.72%` children.

## Lever attempted

Candidate lever: cache-blocked batched Float64 value-tape/run-value construction for promoted dense-outer lanes. The candidate built promoted left/right f64 lanes in batches of eight so a descriptor walk could be shared across columns.

The source hunk has been removed. No source change is retained in this commit.

## Isomorphism proof

The attempted hunk was internally isomorphic:

- Output ordering: preserved `plan` order for left run values and `right_positions_csr` bucket order for right tapes.
- Tie-breaking: preserved per-bucket left-position order and right-position CSR order; no hash/RNG path was introduced.
- Floating point: preserved the exact Rust `i64 as f64` cast used by the original `tape_f64` and `left_run_values_f64` closures.
- Null semantics: preserved `NONE_POS` left-missing sentinel and `right_segments` `usize::MAX` right-missing sentinel; zero filler values remained hidden behind the same validity descriptors.
- Golden output: candidate SHA256 matched baseline exactly: `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`; `cmp=0`.

## Benchmark result

Short paired A/B:

- Baseline: `496.0 ms +/- 98.9 ms`.
- Candidate: `494.0 ms +/- 71.3 ms`.
- Ratio: candidate `1.00x +/- 0.25`.

Longer confirmation:

- Baseline: `3.037 s +/- 0.125 s`.
- Candidate: `2.950 s +/- 0.120 s`.
- Ratio: candidate `1.03x +/- 0.06`.

Score: Impact `1` x Confidence `2` / Effort `2` = `1.0`; below the `2.0` keep gate.

## Verdict

Rejected. The profiled benchmark has only one promoted left lane and one promoted right lane, so multi-lane batching does not exercise its intended advantage on this target. Do not repeat the batched value-tape family for this benchmark. The next deeper target should attack a single-lane residual visible in the same profile, such as one-pass null-run validity witness construction or a wider layout primitive that changes the single-lane cost model without repeating the forbidden promoted-i64-as-f64 lazy-lane family.
