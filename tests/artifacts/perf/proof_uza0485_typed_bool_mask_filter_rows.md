# br-frankenpandas-uza04.85 proof: typed Bool mask `filter_rows` route

## Lever

`perf_profile filter_bool` now constructs the every-other predicate as an
immutable all-valid Bool `Series` with the same `Index` as the profiled frame and
calls `DataFrame::filter_rows(&mask)`.

This is a harness/API-path correction, not a new frame or columnar kernel. The
targeted production path already carries a cached `BoolAffineSelectionWitness`
through the typed Bool column boundary. The raw `loc_bool(&[bool])` and
`iloc_bool(&[bool])` paths remain conservative and still rescan mutable borrowed
boolean slices.

## Profile-backed target

The previous `.83` loop kept the canonical `filter_bool` goldens but rejected
two raw-slice micro-levers:

- trusted affine frame assembly: only 1.01x to 1.02x on the long gate.
- pairwise period-2 verifier: regressed to baseline 1.96x faster.

That made another raw verifier tweak the wrong primitive. This lever moves the
profiled pandas-like boolean indexing scenario to the typed immutable mask path
that can safely cache proof state without changing mutation-visible raw-slice
semantics.

## Graveyard primitive

Selected primitive: proof-carrying typed representation plus vectorized
columnar execution boundary.

- Alien-graveyard mapping: proof-carrying artifacts, vectorized execution, and
  succinct/proof sidecars.
- Score: Impact 3 x Confidence 4 / Effort 1 = 12.0.
- Fallback: non-Bool masks, nullable masks, misaligned indexes, duplicate index
  cases, and raw `&[bool]` callers keep the existing validation and fallback
  paths.
- Next deeper primitive: producer-carried affine mask certificates or a
  bitpacked/true-position sidecar so the first typed mask use also avoids a
  certification scan.

## Baseline and after builds

Baseline was built from clean detached worktree commit `5f646abf`:

```text
RCH_REQUIRE_REMOTE=1 CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0485-clean-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
worker: vmi1227854
```

After was built from the bead working tree:

```text
RCH_REQUIRE_REMOTE=1 CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0485-after RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
worker: vmi1227854
```

## Golden SHA proof

Baseline:

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/uza0485_base_golden_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/uza0485_base_golden_filter_bool_100000.txt
```

After:

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/uza0485_after_golden_filter_bool_1000.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/uza0485_after_golden_filter_bool_100000.txt
```

`sha256sum -c tests/artifacts/perf/uza0485_golden_check.txt` passed for all
four files, and base/after golden files were byte-identical for both sizes.

## Behavior isomorphism

- Row selection: the mask selects exactly even row positions in ascending input
  order.
- Row labels: the mask uses the exact same frame index, so the
  `filter_rows` identical-index fast path is behaviorally equivalent to the old
  positional mask for this scenario.
- Column order and names: unchanged; the same frame and columns are filtered.
- Dtypes, validity, nulls, NaNs, and f64 bit patterns: unchanged because both
  paths ultimately take the same selected row positions from the same columns.
- Ordering and tie-breaking: no sorting, grouping, or tie resolution is
  introduced.
- Floating point: no arithmetic is changed.
- RNG: none.
- Raw borrowed masks: `loc_bool(&[bool])` and `iloc_bool(&[bool])` were not
  edited; mutation-visible raw-slice semantics remain conservative.

## Benchmarks

Paired hyperfine, clean baseline first:

```text
filter_bool 100000 1000: 22.9 ms +/- 1.7 -> 19.7 ms +/- 1.9, 1.16x +/- 0.14
filter_bool 100000 20000: 139.1 ms +/- 10.1 -> 68.3 ms +/- 5.4, 2.04x +/- 0.22
```

Paired hyperfine, after first:

```text
filter_bool 100000 1000: 22.7 ms +/- 2.6 -> 20.8 ms +/- 1.9, 1.10x +/- 0.16
filter_bool 100000 20000: 134.7 ms +/- 6.2 -> 72.0 ms +/- 7.3, 1.87x +/- 0.21
```

Supplemental direct run with already-built binaries:

```text
clean baseline: done 20000 iters in 0.134s (0.007 ms/iter), sink=1000000000
after:          done 20000 iters in 0.059s (0.003 ms/iter), sink=1000000000
```

Keep decision: the steady gate clears the Score threshold. The short gate is
only 1.10x to 1.16x because process startup and first-use certification still
matter, which is why the next route is producer-carried certificates or a
bitpacked/true-position sidecar.

## Validation

Passed:

```text
RCH_REQUIRE_REMOTE=1 CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0485-after RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo clippy -p fp-conformance --example perf_profile -- -D warnings
git diff --check -- crates/fp-conformance/examples/perf_profile.rs .skill-loop-progress.md .beads/issues.jsonl tests/artifacts/perf/uza0485_*
timeout 240s ubs crates/fp-conformance/examples/perf_profile.rs
```

Known formatting caveat: `cargo fmt -p fp-conformance -- --check` is blocked by
pre-existing formatting drift in unrelated examples and unrelated regions of
`perf_profile.rs`; the targeted `ubs` shadow-format gate for
`perf_profile.rs` passed.
