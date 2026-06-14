# br-frankenpandas-x8mdu: cached contiguous UTF-8 factorize witness keep

## Verdict

KEEP.

One lever landed: immutable all-valid `LazyContiguousUtf8` columns now cache a
default factorize witness with:

- `codes: Arc<[i64]>`
- `unique_bytes: Arc<[u8]>`
- `unique_offsets: Arc<[usize]>`

The witness is used only for default factorize semantics:
`sort=false, use_na_sentinel=true`, all-valid `DType::Utf8`, and exact
`LazyContiguousUtf8` backing. Nullable, sorted, non-default, categorical,
object-backed, gather/slice, and lower-hex paths keep the prior implementation.

Score: Impact 4 x Confidence 4 / Effort 2 = 8.0.

## Baseline

Current-head baseline was built before the candidate:

```text
CARGO_TARGET_DIR=.rch-target-lavenderstone-vcstr-current \
RUSTFLAGS='-C force-frame-pointers=yes' \
rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Baseline matrix artifact:
`tests/artifacts/perf/lavender_vcstr_current_cardinality_x50.json`

Current-head target timing:

```text
str_factorize 500000 50: 398.8 ms +/- 22.6 ms
```

Profiler status:

- `perf` and `samply` were blocked by `perf_event_paranoid=4`.
- `strace -c` on `str_factorize 500000x5` showed 110 syscalls and 0.001324 s
  total syscall time, so the residual was not syscall-bound.
- Prior `a3y5o` byte-span per-call factorize was rejected on the x50 gate,
  which routed this pass to a reusable cached witness rather than another
  per-call hash-loop tweak.

Design artifact:
`tests/artifacts/perf/proof_x8mdu_utf8_factorize_witness_design.md`
sha256 `a77fc4aac80e24e37cf18a8c52689b3ab926ad054f99969dbc3f205c786b614f`.

## Candidate Build

Candidate build command:

```text
CARGO_TARGET_DIR=.rch-target-lavenderstone-x8mdu-after \
RUSTFLAGS='-C force-frame-pointers=yes' \
rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

RCH had no admissible worker for this build and failed open locally:

```text
[RCH] local (no admissible workers: insufficient_slots=1,hard_preflight=10,active_project_exclusion=1)
Finished `release-perf` profile [optimized + debuginfo] target(s) in 2m 25s
```

This stayed crate-scoped to `fp-conformance`'s `perf_profile` example.

## Golden Output

Commands:

```text
.rch-target-lavenderstone-vcstr-current/release-perf/examples/perf_profile golden str_factorize 1000
.rch-target-lavenderstone-x8mdu-after/release-perf/examples/perf_profile golden str_factorize 1000
.rch-target-lavenderstone-vcstr-current/release-perf/examples/perf_profile golden str_factorize 500000
.rch-target-lavenderstone-x8mdu-after/release-perf/examples/perf_profile golden str_factorize 500000
```

`cmp -s` passed for both sizes.

Golden sha256:

```text
cd726ef905573f7e869818443fb3320d49ecf7fbc0556ca2d2d6794743491fc4  str_factorize_1000
da2da85584bdca84af3055c9104c531d5bdf760b191420e8aa5f8366cfd3fe11  str_factorize_500000
```

Artifacts:

- `tests/artifacts/perf/x8mdu_base_golden_sha256.txt`
- `tests/artifacts/perf/x8mdu_after_golden_sha256.txt`

## Benchmark Gate

Forward paired gate:

```text
baseline:  381.0 ms +/- 14.0 ms
candidate:  85.4 ms +/-  6.9 ms
speedup: 4.46x +/- 0.40x
artifact: tests/artifacts/perf/x8mdu_pair_factorize_500k_x50_forward.json
sha256: 512cabf68d5cdc64325e202b2a68e316328d30e89e25531da8ff36b3f10372b7
```

Reversed paired gate:

```text
candidate:  83.4 ms +/-  4.1 ms
baseline:  379.2 ms +/- 13.2 ms
speedup: 4.55x +/- 0.28x
artifact: tests/artifacts/perf/x8mdu_pair_factorize_500k_x50_reversed.json
sha256: b73ffff7a14db8c3a986fc141eb1b379ca577fb65943d1b48f45cb740206dc82
```

## Isomorphism Proof

Ordering:

- Witness construction walks rows in source order.
- First sight of each byte span assigns `unique_offsets.len() - 1`, matching the
  previous first-seen factorize rule.
- Repeated rows reuse the first assigned code.

Tie behavior:

- Equality is exact byte-slice equality over `&bytes[offsets[i]..offsets[i+1]]`.
- No sorting occurs on the cached path.
- `sort=true` remains on the existing fallback path and is covered by the new
  lazy-contiguous unit test.

Missing semantics:

- The cached path requires `validity.all()`.
- Nullable, null-sentinel, and `use_na_sentinel=false` modes keep the existing
  fallback path.

Dtype and labels:

- Codes remain `DType::Int64`.
- Uniques remain `DType::Utf8`.
- `Series::factorize()` still builds the same `0..len` code index and
  `0..nuniques` unique index.

Floating point and RNG:

- Not applicable. This path is byte-exact UTF-8 factorize and does not touch
  floating-point arithmetic or randomness.

## Validation

Passed:

```text
rch exec -- cargo check -p fp-columnar --lib
rch exec -- cargo check -p fp-frame --lib
rch exec -- cargo test -p fp-columnar factorize_lazy_contiguous_utf8_default_witness_preserves_order --lib -- --nocapture
rch exec -- cargo test -p fp-columnar factorize --lib -- --nocapture
rch exec -- cargo test -p fp-frame factorize --lib -- --nocapture
rch exec -- cargo clippy -p fp-columnar --lib -- -D warnings
rustfmt --edition 2024 --check crates/fp-columnar/src/lib.rs
git diff --check -- crates/fp-columnar/src/lib.rs crates/fp-frame/src/lib.rs .skill-loop-progress.md tests/artifacts/perf/proof_x8mdu_utf8_factorize_witness_design.md tests/artifacts/perf/x8mdu_*
ubs crates/fp-columnar/src/lib.rs
```

Focused test results:

- `fp-columnar factorize`: 8 passed, 0 failed, 1 ignored.
- `fp-frame factorize`: 10 passed, 0 failed.
- Lazy witness regression: 1 passed, 0 failed.
- `fp-columnar` UBS: exit 0, critical 0.

Note: the final refreshed `fp-frame factorize` command was invoked through
`rch exec`, but RCH had no admissible worker and failed open locally. An earlier
remote `fp-frame factorize` run also passed before the source hunk was
re-applied after concurrent edits.

Blocked or scoped out:

- `rch exec -- cargo clippy -p fp-frame --lib -- -D warnings` is blocked by a
  pre-existing `clippy::large_enum_variant` finding at
  `crates/fp-frame/src/lib.rs:33850` on `SeriesResetIndexResult`.
- Full `rustfmt --check` over both touched source files reports unrelated
  formatting drift in existing `fp-frame` sections outside this factorize hunk.
  `rustfmt --check crates/fp-columnar/src/lib.rs` and `git diff --check` passed.
- `ubs crates/fp-frame/src/lib.rs` timed out after 120 seconds; the file is
  very large and unrelated broad inventories are already known. The
  `fp-columnar` changed file completed with exit 0.

## Next Residual

Post-keep re-profile artifact:
`tests/artifacts/perf/x8mdu_after_cardinality_x50.json`.

```text
str_factorize 500000x50:     87.4 ms +/-  3.8
str_unique 500000x50:       296.9 ms +/-  6.8
str_duplicated 500000x50:   376.6 ms +/-  7.4
str_value_counts 500000x50: 313.7 ms +/- 16.1
```

The next profile-backed bead is `br-frankenpandas-uza04.119`:
`[perf] Reuse contiguous UTF-8 witness for repeated duplicated masks`.

The expected next target is one of:

- direct-indexed counts from the cached factorize witness for repeated
  `str_value_counts` or duplicate masks, if the next profile shows string
  cardinality remains top;
- a different profile-backed primitive from the next ready `[perf]` bead.
