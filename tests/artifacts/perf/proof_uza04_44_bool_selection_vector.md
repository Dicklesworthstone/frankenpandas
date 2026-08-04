# br-frankenpandas-uza04.44 proof

## Target

- Bead: `br-frankenpandas-uza04.44`
- Candidate lever: replace `DataFrame::loc_bool`'s iterator `filter_map().collect()` position build with a safe, unrolled, capacity-aware boolean-mask scan.
- Profile-backed reason: `.44` before-profile for `filter_bool 100000x1000` showed the mask-position collection path under `DataFrame::loc_bool` at 18.58% children, with total `loc_bool` still material in the profile after the `.41` kept lazy-strided Float64 projection.

## Isomorphism proof

- Golden commands covered `filter_bool 1000` and `filter_bool 100000`.
- Before and after golden outputs are byte-identical.
- Verified hashes:
  - `filter_bool 1000`: `f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c`
  - `filter_bool 100000`: `2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea`
- Preserved contracts: row order, output index labels, index name, column order and names, dtype and validity/null/NaN behavior, and f64 payload bits.
- Duplicate behavior: not applicable to a boolean mask; each row position is visited once and emitted at most once.
- Tie-breaking: not applicable.
- RNG: not used.

## Benchmarks

Same-machine paired hyperfine compared fresh before/after `release-perf` binaries.

### Baseline-only timing

- `filter_bool 100000 20`: 46.3 ms +/- 2.7 ms
- `filter_bool 100000 1000`: 465.3 ms +/- 11.7 ms

### Paired `filter_bool 100000 20`

- Before: 53.5 ms +/- 5.4 ms
- After: 49.2 ms +/- 2.8 ms
- Ratio: 1.09x +/- 0.13 faster

### Paired `filter_bool 100000 1000`

- First pair:
  - Before: 507.2 ms +/- 26.4 ms
  - After: 443.0 ms +/- 28.4 ms
  - Ratio: 1.14x +/- 0.09 faster
- Confirming pair with more warmup/runs:
  - Before: 470.5 ms +/- 15.6 ms
  - After: 425.7 ms +/- 6.7 ms
  - Ratio: 1.11x +/- 0.04 faster

## Profile delta

- Before: iterator `filter_map().collect()` selection-vector path under `loc_bool`: 18.58% children.
- After: `boolean_mask_positions` path under `loc_bool`: 8.56% children.
- Remaining top target after this keep is still per-column regular-position detection in `Column::take_positions` / `arithmetic_progression_positions`, not this mask builder.

## Validation

- `rch exec -- cargo test -p fp-frame dataframe_loc_bool --lib -- --nocapture`: passed, 5 tests.
- `rch exec -- cargo check -p fp-frame --lib`: passed.
- `rch exec -- cargo clippy -p fp-frame --lib -- -D warnings`: passed.
- `git diff --check -- crates/fp-frame/src/lib.rs tests/artifacts/perf/proof_rejected_uza04_43_arithmetic_index.md`: passed.
- `rustfmt --edition 2024 --check crates/fp-frame/src/lib.rs`: blocked by pre-existing formatting drift elsewhere in `fp-frame`.
- `timeout 120s ubs crates/fp-frame/src/lib.rs`: timed out with no findings emitted.

## Decision

Kept.

Score: `Impact 2 x Confidence 2 / Effort 1 = 4.0`, above the required `>= 2.0` keep gate. The win is not huge, but it is profile-local, repeat-confirmed, byte-identical, and low-risk.

## Next route

Re-profile after this keep. The shifted bottleneck is still the per-column regular-position scan in `Column::take_positions` / `arithmetic_progression_positions`; do not repeat the `.42` shared-descriptor family. The next primitive should be structurally different, for example a columnar batch/filter primitive that avoids per-column AP rediscovery by changing the row-selection execution model rather than passing a generic descriptor through the existing take path.
