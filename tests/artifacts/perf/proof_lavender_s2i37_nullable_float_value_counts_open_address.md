# br-frankenpandas-s2i37 nullable Float64 value_counts open-address tally

Base: `origin/main` `2e2bed95`.

## Target

Profile-backed tail from `br-frankenpandas-s2i37`: `Series::value_counts()` on typed nullable `Float64` (`1M`, `float64_nan50`) still spent most time in the `FxHashMap<u64, usize>` bit tally for roughly 500k valid high-cardinality values. Lever: replace only that nullable Float64 key-to-index table with a safe Rust power-of-two linear-probe table. All-valid Float64 and non-float paths are unchanged.

RCH note: `rch exec` classified the crate-scoped commands correctly. The
timing commands ran through RCH local fallback when no admissible worker was
available; later crate-scoped `check`/`clippy` gates also ran through RCH, with
some accepted by worker `vmi1227854`.

## Timing

Command:

```bash
env -u CARGO_TARGET_DIR RCH_VERBOSE=1 rch exec -- cargo run --profile release-perf -p fp-bench -- --category dataframe_ops --workload value_counts --size 1M --dtype float64_nan50 --json
```

`fp-bench` internal timings, current-main baseline:

- mean `91632.299800 us`
- median `90391.719 us`
- min/max `75349.576 / 108814.405 us`
- JSON SHA256 `4fe4b8cdef0519cb28c0e3cb7d7b37758b0fe388b7f7ca83134c83d8dd003b6a`
- artifact `lavender_s2i37_vc_currentmain_base_fp_bench_1m_nan50_fallback.txt`

Candidate:

- mean `73679.864080 us`
- median `71180.741 us`
- min/max `62043.306 / 88079.564 us`
- JSON SHA256 `9de4f6f1b80a1eb1db967bfc69a78877cb7a72ab8310b1101928b048f43b705a`
- artifact `lavender_s2i37_vc_final_candidate_fp_bench_1m_nan50_fallback.txt`

Delta:

- mean speedup `1.2437x`
- median speedup `1.2699x`

Hyperfine over already-built baseline/candidate binaries:

- baseline `2.633 s +/- 0.023 s`
- candidate `2.140 s +/- 0.077 s`
- summary `1.23 +/- 0.05x` faster
- artifacts `lavender_s2i37_vc_final_hyperfine_pair.{txt,json}`

Score: Impact `2.6` (18-21% wall-time cut on the remaining profile-backed nullable value_counts tail) x Confidence `0.9` (same-machine `fp-bench`, hyperfine confirmation, golden SHA match) / Effort `1.0` = `2.34`, keep.

## Isomorphism

Ordering and tie-breaking are preserved: the output vector is still appended only on first sight in row order, and the existing stable descending count sort is unchanged, so equal-count labels keep first-seen order.

Floating-point semantics are preserved: keys still canonicalize `-0.0` and `+0.0` to `0_u64`; nonzero values use exact `f64::to_bits()`; the first-seen payload value is stored in `Float64ValueCount.value` exactly as before. NaN/missing rows are skipped by the same `!validity.get(i)` guard.

RNG is not involved. The deterministic benchmark data generator and output materializer are unchanged.

Golden output:

```text
5c220bfe9574c49dd3755f183f90b0f9257489875fe36745313b2f89b5e58cc4  lavender_s2i37_vc_golden_base_nullable_value_counts_5000.txt
5c220bfe9574c49dd3755f183f90b0f9257489875fe36745313b2f89b5e58cc4  lavender_s2i37_vc_golden_candidate_nullable_value_counts_5000.txt
MATCH nullable_float_value_counts_5000
```

Focused unit proof: `series_value_counts_nullable_float_open_addressed_tally_s2i37` verifies nullable typed path selection, missing-row exclusion, zero canonicalization, and first-seen tie order.

## Validation

- `cargo fmt -p fp-frame -p fp-conformance --check`: pass
- `cargo test -p fp-frame series_value_counts_nullable_float_open_addressed_tally_s2i37 -- --nocapture`: pass
- `cargo check -p fp-frame --all-targets`: pass
- `cargo check -p fp-conformance --example perf_profile`: pass
- `cargo clippy -p fp-frame --all-targets -- -D warnings`: pass
- `cargo clippy -p fp-conformance --example perf_profile -- -D warnings`: pass
- `ubs crates/fp-frame/src/lib.rs crates/fp-conformance/examples/perf_profile.rs`: timed out/stuck in UBS Rust `ast-grep` unwrap scan after more than five minutes; see `lavender_s2i37_vc_ubs_touched.txt`
