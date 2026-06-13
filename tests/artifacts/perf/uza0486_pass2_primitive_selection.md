# br-frankenpandas-uza04.86 Pass 2 Primitive Selection

## Recommendation Contract

Change:
Seed all-valid Bool mask affine-selection witnesses at the producer boundary.
For the measured `filter_bool` benchmark, build the every-other mask with a
constructor that already knows `start=0`, `step=2`, and `len=ceil(n/2)`, so
`DataFrame::filter_rows` can reuse `Column::bool_affine_selection_witness()`
without first scanning the full immutable Bool backing. Keep raw
`loc_bool(&[bool])` and `iloc_bool(&[bool])` unchanged.

Hotspot evidence:
`.85` proved repeated typed-mask filtering clears the long gate after the
witness cache is warm, but short gates stayed only `1.10x..1.16x`. Pass 1
baseline on current HEAD measured:

- `filter_bool 100000 1000`: `19.6 ms +/- 1.7 ms`
- `filter_bool 100000 20000`: `65.1 ms +/- 3.8 ms`
- canonical goldens unchanged:
  - `f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c`
  - `2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea`

Mapped graveyard sections:

- `alien_cs_graveyard.md` §8.2 Vectorized Execution: selection vectors and
  typed column batches are the right abstraction for filters. This lever moves
  the selection proof to the typed mask representation boundary.
- `high_level_summary...md` §14 Proof-Carrying Artifacts: the Bool mask carries
  a small deterministic certificate (`start`, `step`, `len`) that hot consumers
  can trust after construction-time bounds checks.
- `alien_cs_graveyard.md` §5.13 AARA-style resource certificates: this is a
  local coefficient-level resource proof. The mask producer certifies that the
  selection is affine, avoiding a later O(n) rediscovery scan.

EV score:

- Impact: 2. The lever only removes first-use certification and does not yet
  remove dense Bool allocation.
- Confidence: 4. The current call chain already consumes cached witnesses; the
  proof is structurally simple.
- Effort: 1. Small constructor + harness route.
- Score: `(2 * 4) / 1 = 8.0`, above the `2.0` gate.

Relevance score:

- Symptom fit: 4/5
- Architecture fit: 5/5
- Proof readiness: 5/5
- Operability: 5/5
- Weighted relevance: `4.7/5`

## Budgeted Mode And Fallback

Budget:
The constructor must perform O(selection_len) writes and O(1) proof validation.
It must fail closed if `step == 0`, if checked arithmetic overflows, or if the
last selected row is outside the mask length.

Fallback:
Invalid descriptors return `None`. Generic Bool producers continue to use
`Column::from_bool_values`, which still scans on the first witness query.
Nullable Bool masks, non-Bool masks, misaligned indexes, and duplicate-index
alignment paths keep existing fallback behavior. Raw borrowed slices remain
mutation-visible and conservative.

## Isomorphism Proof Plan

- Ordering preserved: affine certificate selects `start + i * step` in
  increasing `i`, exactly the same true positions as the dense Bool mask.
- Tie-breaking unchanged: filtering has no tie-breaker; selected row order is
  unchanged.
- Floating-point unchanged: payload columns are gathered through existing
  affine row-selection paths; f64 bits are not recomputed.
- RNG unchanged: no RNG.
- Golden outputs: verify `filter_bool 1000` and `filter_bool 100000` against
  the Pass 1 SHA files after the change.
- Fallback/error behavior: raw `loc_bool(&[bool])`, raw `iloc_bool(&[bool])`,
  nullable masks, duplicate-index masks, and invalid descriptors keep existing
  error/fallback behavior.

## Alternatives Rejected

Full `LazyAffineBool` representation:
Higher ceiling because it removes dense Bool storage entirely, but it touches
more `ScalarValues` arms (`as_slice`, `len`, clone, debug/serde, bool slice
semantics) and changes the representation contract more broadly. Keep it as the
next primitive if this seeded-witness lever is proof-clean but below the score
gate.

Raw-slice verifier rewrites:
Rejected by `.83` and `.77`; they either regressed or failed paired gates, and
they cannot preserve mutation-visible borrowed-slice semantics while caching a
proof.

Frame-constructor bypass:
Rejected by `.76` and `.83`; the constructor is not the current measured wall.

Bitpacked sidecar:
Useful for memory traffic, but broader than needed for the first-use affine
certificate residual and would require additional scalar materialization
surfaces. Defer until a profile points at mask storage bandwidth rather than
certificate rediscovery.
