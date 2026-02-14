# Round 3 Opportunity Matrix

| Hotspot | Impact (1-5) | Confidence (1-5) | Effort (1-5) | Score | Decision |
|---|---:|---:|---:|---:|---|
| `fp-groupby::groupby_sum` unconditional alignment/reindex on already equal indexes | 5 | 5 | 1 | 25.0 | implement |
| `fp-groupby::groupby_sum` key-path clone pressure in map insertion | 3 | 4 | 2 | 6.0 | queue |
| `fp-index::has_duplicates` repeated hash-based scan in fast path guard | 3 | 4 | 2 | 6.0 | queue |
| `RandomState` hashing overhead for hot ephemeral maps (`groupby`/`align`) | 3 | 3 | 3 | 3.0 | queue |

Applied lever this round:

- guarded identity-alignment fast path in `groupby_sum`:
  - if `keys.index() == values.index()` and index has no duplicates, consume original value slices directly;
  - otherwise preserve full `align_union + reindex` path.

Reasoning:

- highest EV candidate from measured workload;
- preserves current duplicate-index semantics by keeping old path when duplicates exist;
- no API or output-contract changes in scoped conformance matrix.

