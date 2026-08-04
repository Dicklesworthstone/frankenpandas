# br-frankenpandas-yq96z: dense-cycle LEFT join materialization proof

## Target

- Bead: `br-frankenpandas-yq96z`
- Profile-backed hotspot: large repeated-key `left_join`/`outer_join` materialization at `n=500000`.
- Lever: add a descriptor-free lazy typed `Int64` materialization path for certified dense-cycle LEFT joins. The older generic/dense LEFT route remains the fallback.

## Baseline

Built via:

```text
CARGO_TARGET_DIR=.rch-target-yq96z-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Hyperfine:

```text
left_join 500000 1:   117.1 ms +/- 8.1 ms
outer_join 500000 1:   81.9 ms +/- 5.5 ms
left_join 500000 20:  704.4 ms +/- 21.5 ms
outer_join 500000 20:  83.4 ms +/- 7.2 ms
```

Goldens:

```text
left_join 5000:  745e5a467cb832077473c38f724f7082cb0ddbf178e92d74d6f462ac73322095
outer_join 5000: d774e320e0fa750b37c62a62c38aee2c905b97c0cf2fad0a3d0e78cf9919c266
```

## After

Built via:

```text
CARGO_TARGET_DIR=.rch-target-yq96z-work RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Hyperfine:

```text
left_join 500000 1:    83.9 ms +/- 10.6 ms
outer_join 500000 1:   80.5 ms +/- 9.5 ms
left_join 500000 20:  109.4 ms +/- 6.4 ms
outer_join 500000 20:  76.8 ms +/- 6.5 ms
```

Delta:

```text
left_join 500000 1:   1.40x faster
left_join 500000 20:  6.44x faster
outer_join control: unchanged/slightly faster within noise
```

Initial isolated score:

```text
Impact 6 x Confidence 5 / Effort 3 = 10.0; keep
```

## Current-origin rebase check

Upstream landed a generic dense-cycle probe primitive while this pass was in
flight. The accepted gate for this commit is therefore the incremental result
against current `origin/main` (`6c7543cb`) rebuilt in a clean baseline worktree,
with this branch rebuilt separately:

```text
origin/main: /data/projects/.scratch/frankenpandas-yq96z-origin-baseline-20260613-180636/.rch-target-yq96z-origin-baseline/release-perf/examples/perf_profile
candidate:   .rch-target-yq96z-candidate-rebase/release-perf/examples/perf_profile
```

Hyperfine:

```text
left_join 500000 20: origin/main 133.1 ms +/- 5.2 ms -> candidate 106.3 ms +/- 8.3 ms (1.25x faster)
left_join 500000 1:  origin/main  83.1 ms +/- 7.6 ms -> candidate  80.6 ms +/- 8.2 ms (1.03x faster/noisy)
outer_join 500000 20 control: origin/main 82.5 ms +/- 5.7 ms -> candidate 79.1 ms +/- 5.9 ms (not regressed)
```

Rebase goldens:

```text
left_join 5000:  origin/main 745e5a467cb832077473c38f724f7082cb0ddbf178e92d74d6f462ac73322095
left_join 5000:  candidate   745e5a467cb832077473c38f724f7082cb0ddbf178e92d74d6f462ac73322095
outer_join 5000: origin/main d774e320e0fa750b37c62a62c38aee2c905b97c0cf2fad0a3d0e78cf9919c266
outer_join 5000: candidate   d774e320e0fa750b37c62a62c38aee2c905b97c0cf2fad0a3d0e78cf9919c266
```

`cmp -s` passed for both current-origin-vs-candidate golden files. Current
rebased score: `Impact 3 x Confidence 4 / Effort 2 = 6.0`; keep.

## Golden equality

After goldens:

```text
left_join 5000:  745e5a467cb832077473c38f724f7082cb0ddbf178e92d74d6f462ac73322095
outer_join 5000: d774e320e0fa750b37c62a62c38aee2c905b97c0cf2fad0a3d0e78cf9919c266
```

`cmp -s` passed for both baseline-vs-after golden files.

## Isomorphism proof

- Dispatch guard: the new route is used only for `JoinType::Left`, one key column, all-valid `Int64` key columns, and both sides carrying `Int64DenseCycleWitness`. All other cases continue through the previous builders.
- Ordering: materialization iterates `left_pos` from `0..left_witness.len`. For each left row it computes the witnessed key from `start + left_pos % period`, then emits either every matching right position `right_offset + k * right_period` in increasing `k`, or exactly one unmatched row. This is the same pandas-observable left-major/right-minor ordering as the existing builder.
- Tie-breaking: duplicate-key fanout order is determined only by dense-cycle witness offset/count and increasing source position. No hash-map iteration, sorting, RNG, or floating-point comparison is introduced.
- Nulls and dtype: unmatched right rows materialize as `Scalar::Null(NullKind::Null)` with the same nullable `Int64` lane contract. Left lanes remain all-valid `Int64`.
- Column order/suffixes: the builder reuses the same merge suffix checks, output names, and insertion order discipline as the surrounding merge code.
- Floating point/RNG: the new path handles typed `Int64` columns only and does not alter floating-point arithmetic or random behavior.

## Validation

```text
cargo fmt -p fp-columnar -p fp-join
cargo fmt --check -p fp-columnar -p fp-join
CARGO_TARGET_DIR=.rch-target-yq96z-work rch exec -- cargo check -p fp-columnar --all-targets
CARGO_TARGET_DIR=.rch-target-yq96z-work rch exec -- cargo check -p fp-join --all-targets
CARGO_TARGET_DIR=.rch-target-yq96z-work rch exec -- cargo clippy -p fp-columnar --lib --tests -- -D warnings
CARGO_TARGET_DIR=.rch-target-yq96z-work rch exec -- cargo clippy -p fp-join --all-targets -- -D warnings
CARGO_TARGET_DIR=.rch-target-yq96z-work rch exec -- cargo test -p fp-join dense_cycle_left_join_output_matches_dense_left_builder
ubs crates/fp-columnar/src/lib.rs crates/fp-join/src/lib.rs
```

Notes:

- `fp-join` clippy all-targets passed.
- `fp-columnar` clippy all-targets is blocked by an unrelated pre-existing example lint in `crates/fp-columnar/examples/bench_str_unique.rs` (`manual_clamp`); `fp-columnar --lib --tests` passed.
- UBS exited nonzero on broad pre-existing inventories and heuristic false positives classifying `DType::Int64`/test key equality as secret comparisons. Cargo fmt/check/clippy/test gates for the changed crates passed as listed above.
- Linux `perf` profiling is blocked in this environment by `perf_event_paranoid=4`; timing proof and goldens are recorded here, and the post-change residual is `left_join 500000x20` at `109.4 ms`.
