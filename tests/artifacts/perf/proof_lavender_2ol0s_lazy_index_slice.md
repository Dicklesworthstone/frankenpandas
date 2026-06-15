# br-frankenpandas-2ol0s proof: lazy index slice + contiguous row slice

## Change

`DataFrame::iloc_slice(start, stop)` now routes single-index DataFrames through a
contiguous row-range path:

- `Index::slice` preserves lazy range/affine/strided representations when
  possible.
- Materialized indexes now carry a shared `Arc<Vec<IndexLabel>>` plus
  `start,len` view instead of cloning labels for every slice.
- Frame columns use existing `Column::take_contiguous_range`.
- MultiIndex frames keep the existing affine/fallback path.

This is one lever: make slice-shaped row selection keep slice-shaped index and
column backing instead of constructing a position vector and cloned index labels.

## Hotspot evidence

`br-frankenpandas-2ol0s` followed the profile-backed `iloc_slice` residual after
the shipped `iloc(list)` affine fix. The earlier direct affine-routing attempt
was behavior-clean but perf-neutral; the remaining O(n) cost was common index and
frame reassembly work rather than column gather alone.

Baseline artifact:

- `tests/artifacts/perf/lavender_2ol0s_base_hyperfine_perf_profile_iloc_slice_100000x50.txt`
- Baseline: `25.6 ms +/- 2.2 ms`

Paired after artifacts:

- `tests/artifacts/perf/lavender_2ol0s_pair_forward_perf_profile_iloc_slice_100000x50.txt`
  - baseline `25.7 ms +/- 1.8 ms`
  - candidate `17.1 ms +/- 1.8 ms`
  - candidate `1.50x +/- 0.19x` faster
- `tests/artifacts/perf/lavender_2ol0s_pair_reversed_perf_profile_iloc_slice_100000x50.txt`
  - candidate `16.6 ms +/- 1.8 ms`
  - baseline `25.9 ms +/- 2.2 ms`
  - candidate `1.56x +/- 0.21x` faster

Score: Impact 4 * Confidence 4 / Effort 2 = 8.0. Accepted.

## Golden SHA256

Before:

```text
a0a28225359ec220a02901f40e2eed1c0b0e53f6cdb24fcffb700d9ebb5e1237  tests/artifacts/perf/lavender_2ol0s_base_golden_iloc_slice_5000.txt
22a1973e9fc0eda8e75a133d70e0b8bf82139bcf3c96559910149e8c91b27463  tests/artifacts/perf/lavender_2ol0s_base_golden_iloc_slice_100000.txt
```

After:

```text
a0a28225359ec220a02901f40e2eed1c0b0e53f6cdb24fcffb700d9ebb5e1237  tests/artifacts/perf/lavender_2ol0s_candidate_golden_iloc_slice_5000.txt
22a1973e9fc0eda8e75a133d70e0b8bf82139bcf3c96559910149e8c91b27463  tests/artifacts/perf/lavender_2ol0s_candidate_golden_iloc_slice_100000.txt
```

Diff artifacts are empty:

- `tests/artifacts/perf/lavender_2ol0s_golden_diff_5000.txt`
- `tests/artifacts/perf/lavender_2ol0s_golden_diff_100000.txt`

## Isomorphism proof

- Ordering preserved: yes. Slice start/stop normalization is unchanged; output
  rows remain the same contiguous row interval in the same order.
- Tie-breaking unchanged: yes. No sorting, hashing, or duplicate resolution is
  introduced.
- Floating-point unchanged: yes. Column values are not recomputed; the path only
  changes how contiguous row backing is selected.
- RNG unchanged: N/A. The operation is deterministic and uses no RNG.
- Index semantics preserved: yes. `Index::slice` clamps to the same interval as
  the old `self.labels[start..end].to_vec()` path. Lazy range/affine/strided and
  materialized-slice views materialize to the same label sequence.
- MultiIndex behavior preserved: yes. MultiIndex frames do not use the new
  direct contiguous path; they continue through the existing affine/fallback
  logic.

## Validation

Passed:

- `rustfmt --edition 2024 --check` on touched Rust files
- `rch exec -- cargo check -p fp-index -p fp-columnar -p fp-frame --lib`
- `rch exec -- cargo clippy -p fp-index -p fp-columnar -p fp-frame --lib -- -D warnings`
- `rch exec -- cargo test -p fp-index slice_of_materialized_index_keeps_shared_label_view --lib`
- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavender-2ol0s-verify rch exec -- cargo test -p fp-frame dataframe_iloc_slice --lib`
- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavender-2ol0s-verify rch exec -- cargo test -p fp-columnar take_contiguous_range_uses_typed_views_without_positions --lib`

RCH note: the two final focused tests fell back locally because no worker was
admissible. A first local fallback hit stale shared `/data/tmp/cargo-target`
proc-macro artifacts; rerunning with the dedicated target directory above passed.

## Risk and rollback

Primary risk: a lazy materialized-slice view could accidentally force callers
that expect an owned materialized vector to share backing longer than before.
Countermeasure: `IndexLabel` is immutable in this representation, `IndexLabels`
still returns read-only slices, clone identity remains fresh, and public behavior
is locked by golden output plus targeted unit tests.

Rollback: revert the source commit for this proof. No fixture behavior changes
are required because before/after goldens are byte-identical.
