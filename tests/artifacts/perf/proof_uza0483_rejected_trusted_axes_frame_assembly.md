# br-frankenpandas-uza04.83 rejection proof - trusted affine frame assembly

Timestamp: 2026-06-12T22:22:00Z
Owner: OrangePeak

## Target

`br-frankenpandas-uza04.83` targeted the post-`.82` `filter_bool` residual:
`DataFrame::loc_bool`, `DataFrame::new_with_axes`, and affine/period-2 mask
verification/frame assembly.

## Candidate Lever

The measured candidate replaced the affine boolean-filter output path's
`Self::new_with_axes(...)` call with a private trusted constructor after the
path had already proved:

- `mask.len() == self.len()`
- affine selected positions are in bounds
- every gathered column either returns a same-length affine output column or
  rejects the path
- row MultiIndex is absent on this fast path
- column names/order/multiindex are cloned from `self`

No source from this candidate was retained because the benchmark did not clear
the keep gate.

## Baseline

Build:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0483-base
RUSTFLAGS='-C force-frame-pointers=yes'
rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Result: remote build passed on `vmi1227854`.

Baseline hyperfine:

```text
filter_bool 100000 1000 mean 22.85045136 ms, stddev 2.35804240 ms
```

Baseline golden SHA:

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  filter_bool 1000
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  filter_bool 100000
```

Fresh local `perf record` was blocked by `perf_event_paranoid=4`; RCH also
refused remote non-compilation `perf`/`cargo flamegraph` commands. Routing
therefore used the bead's attached profile plus the committed profile witness:
steady `filter_bool 100000 20000` dominated by `DataFrame::loc_bool` with
`DataFrame::new_with_axes` visible under affine frame assembly.

## After

Build:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0483-after
RUSTFLAGS='-C force-frame-pointers=yes'
rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Result: remote build passed on `vmi1227854`. The first attempt hit a stale
remote-worker `Cargo.lock` conflict; `rch cache warm --workers vmi1227854`
refreshed the worker cache and the retry passed.

After golden SHA:

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  filter_bool 1000
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  filter_bool 100000
```

`cmp` matched baseline files for both sizes.

## Isomorphism

- Ordering: unchanged; the same affine certificate defines output row order.
- Index labels/name: unchanged; `out_index.rename_index(self.index.name())`
  was still used.
- Columns: unchanged; same `BTreeMap<String, Column>`, same cloned
  `column_order`, same cloned column MultiIndex.
- Dtypes/validity/null/NaN/f64 bits: unchanged; column gathering and all
  typed buffers were untouched. Golden files matched byte-for-byte.
- Tie-breaking: not applicable to boolean row filtering.
- RNG: no RNG surface.
- Fallbacks/errors: non-affine masks and rejected column paths still used the
  existing materialized positions path; wrong mask lengths were unchanged.

## Benchmarks

Short paired gate (`filter_bool 100000 1000`, 20 runs):

```text
base:  24.512 ms +/- 2.032 ms
after: 22.698 ms +/- 1.862 ms
after ran 1.08x +/- 0.13 faster
```

Short reversed gate:

```text
after: 22.761 ms +/- 1.814 ms
base:  21.873 ms +/- 1.611 ms
base ran 1.04x +/- 0.11 faster
```

Long steady gate (`filter_bool 100000 20000`, 12 runs):

```text
base:  134.876 ms +/- 8.450 ms
after: 133.692 ms +/- 4.719 ms
after ran 1.01x +/- 0.07 faster
```

Long reversed gate:

```text
after: 133.317 ms +/- 5.370 ms
base:  136.052 ms +/- 6.870 ms
after ran 1.02x +/- 0.07 faster
```

## Verdict

Rejected. The trusted constructor bypass preserved behavior but did not produce
a real, order-stable win. Score: Impact 1 x Confidence 1 / Effort 1 = 1.0,
below the required `>= 2.0` keep threshold. The production hunk was removed.

Next route: stop attacking frame-constructor validation for this residual. The
profile points deeper at repeated raw `[bool]` mask verification. The next
primitive should be an immutable/typed boolean mask representation or
producer-carried mask plan that lets `loc_bool` consume a proof object without
rescanning ordinary borrowed slices, while preserving mutation-visible `[bool]`
semantics by keeping the existing raw-slice path conservative.
