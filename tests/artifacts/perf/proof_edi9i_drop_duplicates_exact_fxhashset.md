# edi9i — drop_duplicates exact-bits FxHashSet uniqueness (drop the sort) — KEPT (7.6x)

BlackThrush, 2026-06-13. Gap found via the restored fp-bench vs-pandas harness.

## Target (fp-bench vs pandas 2.2.3, drop_duplicates @ 100k float64, all-unique)

```
BEFORE  fp p50 = 3.64 ms   pandas p50 = 1.89 ms   -> 1.93x SLOWER
```

`DataFrame::drop_duplicates(subset=[col], keep=First, ignore_index=false)` on an
all-unique Float64 column hits the unique-shortcut `subset_has_all_valid_unique_column`,
which returns `self.clone()` (Arc-cheap). The whole 3.64 ms was the *uniqueness
check*, whose Float64 arm did:

1. an exact-bit pre-check with `std::collections::HashSet<u64>` — **SipHash**, n
   inserts; then
2. `data.to_vec()` + `sort_by(total_cmp)` — an **O(n log n)** comparison sort; then
3. a `windows(2)` fuzzy-`float_semantic_eq` adjacency walk.

## Lever (one, bit-identical): replace the whole check with one exact-bits FxHashSet pass

```rust
let mut seen = FxHashSet::with_capacity_and_hasher(data.len(), Default::default());
data.iter().all(|&v| seen.insert(v.to_bits()))
```

No sort. SipHash → FxHash. The key insight is that the **fuzzy** (`float_semantic_eq`)
walk was redundant for the *final drop_duplicates output*:

- The only consumer of this shortcut is `drop_duplicates`. When the shortcut is
  declined it falls through to `duplicated(subset, keep)`, which buckets rows by
  the **exact `to_bits` digest** (`Scalar::Float64 => mix(3); mix(to_bits)`).
  Fuzzy-near-but-distinct-bit neighbours land in different buckets and are never
  merged — so `duplicated` performs *exact* dedup.
- Therefore, whenever a column is **exact-unique**, every row is kept by BOTH
  paths: the shortcut clones, and the fallback's "no duplicate marked" arm
  (`!saw_duplicate => self.clone()`) also returns the same all-rows frame.
- A fuzzy-near pair (relative 1e-14, distinct bits) thus only changes *which*
  path produces the identical all-rows-kept frame — never the bytes. An exact
  bit duplicate is still caught here (set insert fails), exactly as before.
- `+0.0` / `-0.0` keep distinct bits here and in the digest path (both keep both
  rows); `as_f64_slice` is all-valid no-NaN, so NaN ordering is moot.

The shortcut is gated on `!ignore_index`, so this arm never runs for
`ignore_index=true`.

## Proof

- Golden `dd_golden` digest **6811f9fc38d507c5** unchanged before/after, over
  four regimes routed through the check: (A) all-unique 100k, (B) exact bit
  duplicates, (C) fuzzy-near-but-distinct (`1.0` vs `1.0+1e-15`), (D) `+0.0`/`-0.0`.
- `cargo test -p fp-frame drop_duplicates`: 18 passed, 0 failed.
- `cargo test -p fp-frame duplicated` / `unique`: passed.
- fp-bench drop_duplicates @ 100k float64, 10 cols (min-of-25, 4 runs):
  **3.64 -> 0.48 ms = 7.6x** — now ~4x FASTER than pandas (1.89 ms).
- The 2-col `dd_golden` shortcut timing corroborates: 3.65 -> 0.47 ms.

## Score

Impact 7.6x x Confidence 0.97 / Effort 1.0 = retained (>= 2.0).

## Rejected on the way here

A radix-argsort replacement of step 2 (keeping the fuzzy walk) was bit-identical
(same digest) but only 1.28x on the 10-col workload: `radix_argsort_f64`'s 8
LSD passes do random `keys[idx[i]]` reads (~2 ms for 100k), barely beating
SipHash+compsort combined. Eliminating the sort entirely (this lever) is the
real win.
