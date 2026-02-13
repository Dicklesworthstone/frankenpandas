# Round 2 Opportunity Matrix

| Hotspot | Impact (1-5) | Confidence (1-5) | Effort (1-5) | Score | Decision |
|---|---:|---:|---:|---:|---|
| `fp-index::align_union` owned-key map construction (`position_map_first`) | 4 | 4 | 1 | 16.0 | implement |
| `fp-join::join_series` cloned-key hash map for right index fanout | 3 | 4 | 2 | 6.0 | queue |
| `fp-conformance::load_fixtures` per-run JSON parse + full directory walk | 3 | 3 | 3 | 3.0 | queue |
| Oracle subprocess spawn amortization for live pandas mode | 4 | 2 | 4 | 2.0 | queue |

Applied lever this round:

- replaced `align_union` map construction with borrowed-key maps (`HashMap<&IndexLabel, usize>`) while preserving output label ordering and alignment semantics.

Why this lever:

- highest EV score in measured path;
- no API-surface breakage;
- straightforward rollback to prior owned-key map path.

