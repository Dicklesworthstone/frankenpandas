# br-frankenpandas-uza04.51 proof: affine filter_bool frame assembly bypass

## Change

`DataFrame::take_rows_by_affine_certificate_unchecked` now constructs the
result `DataFrame` directly after the affine fast path has already proved:

- row multi-index is absent,
- the selected index length is exactly `certificate.len`,
- every output column was produced by the descriptor-only affine column gather,
- `column_order` is cloned unchanged from the source frame, and
- `column_multiindex` is cloned unchanged from the source frame.

This removes a redundant `DataFrame::new_with_axes` pass from the profiled
affine `filter_bool` path. It does not change mask recognition, selected rows,
index projection, column gather, or public dtype/value materialization.

## Profile-backed target

`br-frankenpandas-uza04.51` was filed from a post-`.50` profile:

- `filter_bool 100000 1000`: `42.8 ms +/- 3.3 ms` -> `21.8 ms +/- 1.6 ms`
- shifted residual: `DataFrame::loc_bool` 49.86% self,
  `DataFrame::new_with_axes` 7.29% self, and
  `affine_boolean_mask_span` / `period2_boolean_mask_span` child rows.

Current environment blocks dynamic `perf`:

- `tests/artifacts/perf/uza0451_base_perfstat_filter_bool_100000x20000.txt`
- `tests/artifacts/perf/uza0451_after_perfstat_filter_bool_100000x20000.txt`

Both report `perf_event_paranoid setting is 4`, so timing proof and the existing
profile-backed bead are the available evidence.

## Baseline

Built with:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavenderstone-uza0451-base \
RUSTFLAGS="-C force-frame-pointers=yes" \
rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

RCH had no admissible worker slots and failed open locally; the build remained
crate-scoped and used the isolated target directory above.

Baseline hyperfine:

- `filter_bool 100000 1000`: `20.6 ms +/- 1.6 ms`
- `filter_bool 100000 20000`: `69.4 ms +/- 3.1 ms`
- `filter_bool 100000 60000`: `175.3 ms +/- 8.9 ms`

Artifacts:

- `tests/artifacts/perf/uza0451_current_base_filter_bool_100000x1000.json`
- `tests/artifacts/perf/uza0451_current_base_filter_bool_100000x20000.json`
- `tests/artifacts/perf/uza0451_current_base_filter_bool_100000x60000.json`

## Golden output

Before:

- `filter_bool 1000`: `f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c`
- `filter_bool 100000`: `2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea`

After:

- `filter_bool 1000`: `f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c`
- `filter_bool 100000`: `2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea`

`cmp -s` passed for both before/after golden dumps.

## Paired benchmark result

`filter_bool 100000 20000`:

- forward: baseline `71.2 ms +/- 3.1 ms`, candidate `44.5 ms +/- 3.0 ms`,
  `1.60x +/- 0.13`
- reversed: candidate `44.3 ms +/- 3.0 ms`, baseline `71.1 ms +/- 3.2 ms`,
  `1.61x +/- 0.13`

`filter_bool 100000 60000`:

- forward: baseline `170.9 ms +/- 8.9 ms`, candidate `98.0 ms +/- 7.0 ms`,
  `1.74x +/- 0.15`
- reversed: candidate `98.3 ms +/- 4.3 ms`, baseline `169.2 ms +/- 6.4 ms`,
  `1.72x +/- 0.10`

Artifacts:

- `tests/artifacts/perf/uza0451_pair_filter_bool_100000x20000_forward.json`
- `tests/artifacts/perf/uza0451_pair_filter_bool_100000x20000_reversed.json`
- `tests/artifacts/perf/uza0451_pair_filter_bool_100000x60000_forward.json`
- `tests/artifacts/perf/uza0451_pair_filter_bool_100000x60000_reversed.json`

Score: Impact 3 * Confidence 5 / Effort 1 = 15.0, keep.

## Isomorphism proof

- Ordering preserved: yes. The same affine certificate selects the same row
  positions in ascending certificate order.
- Tie-breaking unchanged: N/A. Boolean filtering does not compare or break ties.
- Floating-point unchanged: yes. The same `Column::take_affine_positions...`
  returns the same lazy strided Float64 descriptors and values; no arithmetic is
  introduced.
- RNG unchanged: N/A.
- Index labels unchanged: yes. `out_index.rename_index(self.index.name())` is
  identical to the prior `new_with_axes` call input.
- Column order unchanged: yes. `self.column_order.clone()` is assigned directly;
  the old normalization would return the same order because the map was built by
  iterating that exact order.
- Column multi-index unchanged: yes. The source multi-index is cloned unchanged,
  and its length already matched the source `column_order`; the fast path does
  not add/drop/reorder columns.
- Duplicate-label policy unchanged: yes. The old `new_with_axes` constructor set
  `allows_duplicate_labels: true`; the direct constructor preserves that exact
  value.
- Failure behavior unchanged for reachable states: yes. The removed
  `new_with_axes` errors required a column length mismatch or column multi-index
  mismatch, both ruled out by the affine fast-path construction invariants.

## Validation

- Focused tests:
  `rch exec -- cargo test -p fp-frame dataframe_loc_bool --lib`
  passed, `6 passed; 0 failed`.
- Crate-scoped check:
  `rch exec -- cargo check -p fp-frame --all-targets`
  passed remotely on `vmi1153651`.
- `cargo fmt -p fp-frame --check` failed on a pre-existing unrelated formatting
  diff at `crates/fp-frame/src/lib.rs:41731`, outside this change.
- `cargo clippy -p fp-frame --all-targets -- -D warnings` failed on the known
  `br-frankenpandas-scowx` test-only lint inventory:
  type-complexity cases around `83178`, `83369`, `83661`, `83877`, `83993`,
  `84131`, and one useless-vec case around `84501`.
- `ubs crates/fp-frame/src/lib.rs` was started as required but hung for more
  than 10 minutes in its suppression helper after the Rust scan banner; it was
  interrupted to avoid leaving a scanner process running. The broad fp-frame UBS
  inventory is already tracked by `br-frankenpandas-yavyk`.
