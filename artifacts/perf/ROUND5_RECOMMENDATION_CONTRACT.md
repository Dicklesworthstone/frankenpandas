# ROUND5 Recommendation Contract

Change:
- Cache duplicate-detection result in `Index` (`OnceCell<bool>`), preserving serde/equality semantics for labels.

Hotspot evidence:
- Round4-after flamegraph showed dominant repeated hash cost in `Index::has_duplicates` and index-label hashing.

Mapped graveyard sections:
- `§7.7` hash-path pressure (avoid redundant hash probes)
- methodology: profile-first, one lever, bounded-risk adoption wedge

EV score:
- `(Impact 5 * Confidence 5 * Reuse 5) / (Effort 1 * Friction 1) = 125.0`

Priority tier:
- `S`

Adoption wedge:
- Internal `fp-index` optimization only; no external API or mode contract changes.

Budgeted mode:
- First call computes duplicate state; subsequent calls use cached value.

Expected-loss model:
- States: `{cache_cold, cache_warm}`
- Actions: `{recompute_each_call, cache_once}`
- Loss: recompute has high repeated CPU loss; cache-once has negligible state-complexity loss.

Calibration + fallback trigger:
- If semantic drift appears, remove cache field and revert to direct scan.

Isomorphism proof plan:
- Unit test ensures equality is label-only regardless of cache state.
- Run groupby/conformance suites and golden checksum verification.

p50/p95/p99 target:
- Reduce tail and median latency by double-digit percentages from round4 baseline comparator.

Primary failure risk + countermeasure:
- Risk: cache field accidentally affecting equality/serialization behavior.
- Countermeasure: manual `PartialEq` implementation on labels + serde skip + dedicated regression test.

Rollback:
- Revert `Index` cache field and `has_duplicates` memoization path.

Baseline comparator:
- Round4 post-change benchmark (`round4_groupby_hyperfine_after.json`).
