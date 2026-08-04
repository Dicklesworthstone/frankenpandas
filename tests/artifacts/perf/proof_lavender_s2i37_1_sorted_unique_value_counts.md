# br-frankenpandas-s2i37.1 proof: nullable Float64 value_counts sorted-unique fast path

## Lever

Add an exact sorted-unique fast path for nullable `Float64` `Series::value_counts`.
The path samples only to decide whether the exact uniqueness check is worth
trying. It still scans all valid rows, sorts canonical bit keys, and falls back
to the existing open-addressed tally whenever any duplicate key exists.

## Benchmark

Workload:

```text
fp-bench --category dataframe_ops --workload value_counts --size 1M --dtype float64_nan50 --json
```

Baseline (`open_base`, current `br-frankenpandas-s2i37` open-addressed tally):

```text
standalone baseline: 2.100 s +/- 0.059 s
paired baseline:     2.084 s +/- 0.036 s
reverse baseline:    2.100 s +/- 0.054 s
```

Candidate (`unique_candidate`, sorted-unique fast path):

```text
paired candidate:    1.366 s +/- 0.023 s
reverse candidate:   1.349 s +/- 0.038 s
paired speedup:      1.53x +/- 0.04
reverse speedup:     1.56x +/- 0.06
```

Score:

```text
Impact 6 x Confidence 0.90 / Effort 2 = 2.70
Decision: keep
```

## Golden output

Fixture:

```text
perf_profile golden value_counts_nan50 5000
```

Baseline and candidate SHA-256:

```text
2e0567f681bdacae8d0b5f59c542582b185b64186ada5bf4352741272dd0afb6
```

Byte compare:

```text
MATCH
```

## Validation

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavender-s2i37-1-test \
  rch exec -- cargo test -p fp-frame \
  series_value_counts_nullable_float_unique_fast_path_matches_generic_s2i37_1 \
  --lib -- --nocapture
PASS

CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavender-s2i37-1-clippy-frame \
  rch exec -- cargo clippy -p fp-frame --lib --tests -- -D warnings
PASS

CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavender-s2i37-1-clippy-conformance \
  rch exec -- cargo clippy -p fp-conformance --example perf_profile -- -D warnings
PASS

rustfmt --check crates/fp-frame/src/lib.rs \
  crates/fp-conformance/examples/perf_profile.rs
PASS

git diff --check -- crates/fp-frame/src/lib.rs \
  crates/fp-conformance/examples/perf_profile.rs \
  tests/artifacts/perf/proof_lavender_s2i37_1_sorted_unique_value_counts.md
PASS

cargo fmt --check
FAIL: pre-existing unrelated rustfmt drift outside this lever

timeout 180 ubs crates/fp-frame/src/lib.rs \
  crates/fp-conformance/examples/perf_profile.rs
TIMEOUT: no findings emitted before timeout
```

## Isomorphism

- Row inclusion is unchanged: nullable `Float64` still skips a row iff
  `!validity.get(i)`. No null/NaN row is converted into a counted value.
- Key identity is unchanged for counted rows: both the fast path and fallback
  use the existing `value == 0.0 -> 0_u64` canonicalization, otherwise
  `value.to_bits()`.
- The sample does not determine output. It only gates whether to try the exact
  check. The exact check sorts every valid canonical key and rejects the fast
  path if any adjacent duplicate is found.
- If a duplicate key exists, the function returns to the existing
  open-addressed tally, preserving first-seen representative value,
  tie-breaking, and stable output order.
- If no duplicate key exists, every valid key has count one. Pandas-compatible
  stable descending count order is therefore the identity over first-seen valid
  row order, and labels/counts match the generic implementation exactly.
- `+0.0` and `-0.0` both canonicalize to `0_u64`; if both occur, the exact
  duplicate check rejects the fast path and falls back, preserving the existing
  representative-value behavior.
- The lever adds no floating-point arithmetic, RNG, parallelism, or unstable
  tie-breaking. It only reads `f64` bit patterns and sorts integer keys for the
  exact uniqueness proof.
