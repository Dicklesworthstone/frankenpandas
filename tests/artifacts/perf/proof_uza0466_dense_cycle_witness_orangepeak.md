# br-frankenpandas-uza04.66 proof - cached dense-cycle Int64 witness

Agent: OrangePeak
Date: 2026-06-10
Target: `perf_profile outer_join 500000`

## Profile-backed target

Fresh current-head baseline after `.65` was rejected:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0466-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Baseline evidence:

- Golden SHA for `outer_join 20000`:
  `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`.
- Hyperfine `outer_join 500000x20`: `267.2 ms +/- 18.3 ms`.
- Direct `outer_join 500000x200`: `1.603 s`, `8.016 ms/iter`.
- Profile: `build_single_key_dense_i64_outer_merge_output` `84.06%` children
  / `24.06%` flat, CSR closure `26.54%` children, tuple plan push `13.83%`
  children, descriptor Arc collection `8.75%` children.

## Lever kept

One safe-Rust lever:

- Add `Int64DenseCycleWitness { start, period, len }`, a proof that an all-valid
  Int64 column satisfies `key[row] == start + (row % period)`.
- Cache that witness in the lazy all-valid Int64 backing with `OnceLock`.
- Make `Column::new(DType::Int64, all-valid scalars)` reuse the existing typed
  `Arc<[i64]>` cache as lazy all-valid Int64 values, so scalar materialization
  and serialization remain byte-for-byte equivalent while the witness cache is
  available to join kernels.
- Let dense outer join build CSR offsets/positions arithmetically from the
  cached witness when present; otherwise fall back to the existing generic CSR
  builder.

This is not the rejected `.59` or `.60` family: those paid periodic proof cost
inside every merge. This lever moves proof to column/first-use metadata and
reuses it across repeated joins.

## Isomorphism proof

- Certification is exact: the first lookup scans every key and rejects empty,
  gapped, overflowed, shifted-wrong, non-periodic, nullable, or non-Int64 input.
- Fallback is unchanged: absent witness routes to the existing CSR builder.
- Output ordering is unchanged: arithmetic CSR emits buckets in ascending key
  order and positions as `offset + k * period`, which is the same increasing
  input-position order produced by the generic cursor fill for a certified
  dense cycle.
- Tie-breaking is unchanged: matched bucket rows still iterate left positions
  first and right CSR segments second.
- Null/NaN semantics are unchanged: nullable Int64 inputs never certify, and
  side-only validity ranges and `NONE_POS` sentinel behavior are untouched.
- Dtype promotion is unchanged: side promotion gates and column constructors are
  unchanged.
- Floating-point behavior is unchanged: no cast site changed; promoted present
  values still use Rust `i64 as f64`.
- RNG/hash behavior is unchanged; the optimized path is deterministic arithmetic
  and does not add hash iteration or randomness.

Golden after SHA:

```text
453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750
```

Baseline and after goldens matched byte-for-byte (`golden_cmp=0`).

## Benchmark gate

Paired `outer_join 500000x20`:

- Baseline: `269.7 ms +/- 9.7 ms`.
- After: `230.7 ms +/- 21.2 ms`.
- Ratio: after `1.17x +/- 0.12` faster.

Paired `outer_join 500000x200`:

- Baseline: `1.624 s +/- 0.053 s`.
- After: `1.242 s +/- 0.060 s`.
- Ratio: after `1.31x +/- 0.08` faster.

Direct after `outer_join 500000x200`: `1.349 s`, `6.747 ms/iter`.

Score: Impact `4` x Confidence `5` / Effort `3` = `6.67`; keep.

## Validation

Passed:

```text
cargo fmt -p fp-columnar -p fp-join -- --check
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0466-test-columnar rch exec -- cargo test -p fp-columnar int64_dense_cycle_witness --lib
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0466-test-join rch exec -- cargo test -p fp-join merge_outer_dense_int64_duplicates_matches_generic_validated_route --lib
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0466-check rch exec -- cargo check -p fp-columnar -p fp-join --all-targets
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0466-clippy2 rch exec -- cargo clippy -p fp-columnar -p fp-join --all-targets --no-deps -- -D warnings
git diff --check
```

UBS over the two touched Rust files exited non-zero on the existing broad
repository inventory and false-positive security matches (`DType::Int64` and
`NONE_POS` comparisons). No UBS finding targeted the new witness verifier or
arithmetic CSR logic.

## Shifted residual

After profile:

- `build_single_key_dense_i64_outer_merge_output`: `81.44%` children.
- Witness arithmetic CSR position emission: `15.39%` children.
- Plan tuple push path: `12.23%` children.
- Descriptor Arc collections: `10.54%`, `8.45%`, and `8.24%` children.

The original generic key-scan CSR closure is gone from the hot list. The next
route should consume the cached witness more deeply by building dense-cycle run
descriptors directly from witness arithmetic, avoiding the plan tuple and
descriptor post-pass while not repeating generic descriptor remapping from
`.54`/`.65`.
