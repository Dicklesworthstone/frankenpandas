# ESSENCE_EXTRACTION_LEDGER.md — FrankenPandas Phase-2C Foundation

Date: 2026-02-14
Bead: `bd-2gi.1`
Status: in-progress foundation ledger (initial populated coverage for FP-P2C-001..005)

## 0. Purpose

This is the canonical essence-extraction ledger for Phase-2C packet execution.
It centralizes the packet-level behavior essence required by clean-room Rust reimplementation:

1. legacy anchors,
2. behavioral invariants,
3. hidden assumptions,
4. undefined-behavior edges,
5. strict/hardened divergence rules,
6. explicit non-goals.

This document is normative for packet planning and implementation sequencing.

## 1. Non-Negotiable Program Contract

- Target is absolute feature/functionality overlap for the scoped packet surface.
- No "minimal v1" reductions are permitted for packet-scoped behavior.
- Implementation is clean-room: spec/oracle extraction first, Rust implementation from extracted contract (no line-by-line translation).
- Unknown incompatible behavior is fail-closed in strict mode and bounded/audited in hardened mode only if explicitly allowlisted.

## 2. Required Ledger Fields (Per Packet)

Each packet row in this document and its packet-local artifacts must maintain:

1. `packet_id`
2. `legacy_paths`
3. `legacy_symbols`
4. `behavioral_invariants`
5. `hidden_assumptions`
6. `undefined_behavior_edges`
7. `strict_mode_policy`
8. `hardened_mode_policy`
9. `explicit_non_goals`
10. `oracle_fixture_mapping`
11. `evidence_artifacts`

## 3. Packet Essence Ledger (Current Coverage)

### FP-P2C-001 — DataFrame/Series construction + alignment

- Legacy anchors:
  - `pandas/core/frame.py` (`DataFrame`, `_from_nested_dict`, `_reindex_for_setitem`)
  - `pandas/core/series.py` (`Series` constructor/alignment arithmetic)
- Behavioral invariants:
  - label-driven deterministic union materialization before arithmetic,
  - missing labels emit missing results rather than row drops,
  - duplicate-label handling is mode-gated and auditable,
  - alignment uses left-order-preserving union with right-unseen append.
- Hidden assumptions:
  - packet handles explicitly labeled scalar series paths only,
  - duplicate handling outside allowlisted path is unsupported in strict mode.
- Undefined-behavior edges:
  - full pandas duplicate-label runtime matrix,
  - full broadcast matrix,
  - full `loc`/`iloc` indexing matrix (deferred packets).
- Strict/hardened divergence:
  - strict: fail-closed on unsupported duplicate semantics,
  - hardened: bounded repair only with mandatory decision/evidence logging.
- Explicit non-goals:
  - full duplicate-label matrix,
  - advanced broadcast/indexing beyond scoped surface.
- Sources:
  - `artifacts/phase2c/FP-P2C-001/legacy_anchor_map.md`
  - `artifacts/phase2c/FP-P2C-001/contract_table.md`
  - `artifacts/phase2c/FP-P2C-001/risk_note.md`

### FP-P2C-002 — Index model + indexer semantics

- Legacy anchors:
  - `pandas/core/indexes/base.py` (`Index`, `ensure_index`, `_validate_join_method`)
- Behavioral invariants:
  - deterministic union with first-occurrence positional map,
  - invalid alignment vectors fail explicitly,
  - ordering preserved for implemented union paths.
- Hidden assumptions:
  - null semantics are out of scope for this index packet,
  - duplicate edge handling is constrained to explicit fixture families.
- Undefined-behavior edges:
  - `MultiIndex` semantics,
  - partial-string index slicing,
  - timezone-specific index semantics.
- Strict/hardened divergence:
  - strict: fail-closed on unsupported surfaces,
  - hardened: allowlisted bounded repair with decision ledger entries.
- Explicit non-goals:
  - `MultiIndex`,
  - timezone and partial-string index semantics.
- Sources:
  - `artifacts/phase2c/FP-P2C-002/legacy_anchor_map.md`
  - `artifacts/phase2c/FP-P2C-002/contract_table.md`
  - `artifacts/phase2c/FP-P2C-002/risk_note.md`

### FP-P2C-003 — Series arithmetic alignment + duplicate-label behavior

- Legacy anchors:
  - `pandas/core/series.py` aligned binary arithmetic paths,
  - `pandas/core/indexes/base.py` index union/duplicate behavior.
- Behavioral invariants:
  - deterministic label union before arithmetic,
  - non-overlap labels map to missing outputs,
  - duplicate paths are explicit/auditable and mode-gated,
  - left-order-preserving union plus right-unseen append.
- Hidden assumptions:
  - unsupported compatibility surfaces stay fail-closed,
  - hardened duplicate repairs remain bounded and explicitly logged.
- Undefined-behavior edges:
  - full duplicate-label runtime matrix,
  - advanced broadcast semantics.
- Strict/hardened divergence:
  - strict: reject unsupported/unknown surfaces,
  - hardened: bounded allowlisted repairs with mismatch corpus emission.
- Explicit non-goals:
  - full duplicate-label semantics,
  - advanced broadcast semantics.
- Sources:
  - `artifacts/phase2c/FP-P2C-003/legacy_anchor_map.md`
  - `artifacts/phase2c/FP-P2C-003/contract_table.md`
  - `artifacts/phase2c/FP-P2C-003/risk_note.md`

### FP-P2C-004 — Join semantics (indexed series join core)

- Legacy anchors:
  - `pandas/core/reshape/merge.py` (`merge`, `_MergeOperation`, indexer semantics),
  - `pandas/core/series.py` indexed join behavior.
- Behavioral invariants:
  - inner join with duplicate keys expands to deterministic cross-product cardinality,
  - left join preserves left ordering and inserts missing right values,
  - duplicate expansion order is stable/nested-loop deterministic.
- Hidden assumptions:
  - scoped packet covers `inner`/`left` indexed series join family only,
  - unknown join modes are fail-closed.
- Undefined-behavior edges:
  - full DataFrame multi-column merge matrix,
  - full sort semantics matrix.
- Strict/hardened divergence:
  - strict: fail-closed on unknown semantics,
  - hardened: bounded continuation only with decision logging.
- Explicit non-goals:
  - multi-column merges,
  - non-scoped join mode/sort matrix.
- Sources:
  - `artifacts/phase2c/FP-P2C-004/legacy_anchor_map.md`
  - `artifacts/phase2c/FP-P2C-004/contract_table.md`
  - `artifacts/phase2c/FP-P2C-004/risk_note.md`

### FP-P2C-005 — Groupby planner + sum aggregate core

- Legacy anchors:
  - `pandas/core/groupby/groupby.py`,
  - `pandas/core/groupby/ops.py`,
  - series-groupby entry points.
- Behavioral invariants:
  - deterministic group key encounter order when `sort=false`,
  - `dropna=true` behavior preserved for key inclusion policy,
  - missing values do not contribute to sums.
- Hidden assumptions:
  - packet scoped to `sum` aggregate only,
  - incompatible payload shapes fail early.
- Undefined-behavior edges:
  - multi-aggregate matrix (`mean`, `count`, `min`, `max`, etc.),
  - multi-key DataFrame groupby matrix.
- Strict/hardened divergence:
  - strict: zero critical drift tolerated,
  - hardened: divergence only in explicit allowlist with ledger hooks.
- Explicit non-goals:
  - non-sum aggregate matrix,
  - multi-key DataFrame groupby semantics.
- Sources:
  - `artifacts/phase2c/FP-P2C-005/legacy_anchor_map.md`
  - `artifacts/phase2c/FP-P2C-005/contract_table.md`
  - `artifacts/phase2c/FP-P2C-005/risk_note.md`

### FP-P2C-006 — Join + concat semantics (provisional until packet artifacts land)

- Legacy anchors:
  - `pandas/core/reshape/merge.py` (`merge`, `_MergeOperation`, `get_join_indexers`)
  - `pandas/core/reshape/concat.py` (`concat`, `_get_result`, `_make_concat_multiindex`)
- Behavioral invariants (provisional):
  - join cardinality must match key multiplicity semantics for each join mode in scope,
  - concat axis semantics preserve declared ordering/promotion rules,
  - null-side handling is deterministic and explicit by mode.
- Hidden assumptions (provisional):
  - first implementation slice may scope join mode matrix and concat axis combinations before full matrix completion.
- Undefined-behavior edges (until extraction complete):
  - full merge/concat option matrix and interaction space.
- Strict/hardened divergence:
  - strict: fail-closed on unknown mode/metadata combinations,
  - hardened: bounded allowlisted defenses only.
- Explicit non-goals (temporary):
  - none at program level; packet-level temporary exclusions must be explicitly listed and retired.
- Sources:
  - `PHASE2C_EXTRACTION_PACKET.md`

### FP-P2C-007 — Missingness + nanops reductions (provisional until packet artifacts land)

- Legacy anchors:
  - `pandas/core/missing.py` (`mask_missing`, `clean_fill_method`, `interpolate_2d_inplace`)
  - `pandas/core/nanops.py` (`nansum`, `nanmean`, `nanmedian`, `nanvar`, `nancorr`)
- Behavioral invariants (provisional):
  - missing propagation remains monotonic under composed operations,
  - NaN/NaT/null distinctions are preserved at observable API boundaries,
  - reduction defaults and numeric coercion are deterministic.
- Hidden assumptions (provisional):
  - dtype-specific missing marker normalization remains centralized in scalar/column contracts.
- Undefined-behavior edges (until extraction complete):
  - full nanops option matrix and edge-case tolerance behavior.
- Strict/hardened divergence:
  - strict: fail-closed on unknown coercion/reduction ambiguity,
  - hardened: explicit bounded recovery only.
- Explicit non-goals (temporary):
  - none at program level; temporary packet exclusions require explicit drift ledger entries.
- Sources:
  - `PHASE2C_EXTRACTION_PACKET.md`

### FP-P2C-008 — IO first-wave contract (provisional until packet artifacts land)

- Legacy anchors:
  - parser/formatter entry points in `pandas/io/*` (CSV + schema normalization first wave).
- Behavioral invariants (provisional):
  - parser normalization is deterministic for scoped dialect/support,
  - malformed input paths are fail-closed with deterministic diagnostics,
  - round-trip stability is preserved for supported schema/value surface.
- Hidden assumptions (provisional):
  - first wave is intentionally scoped while retaining strict compatibility for included IO paths.
- Undefined-behavior edges (until extraction complete):
  - full IO breadth beyond first-wave formats/options.
- Strict/hardened divergence:
  - strict: fail-closed on unsupported metadata/features,
  - hardened: bounded parser recovery for allowlisted corruption classes.
- Explicit non-goals (temporary):
  - none at program level; packet-local exclusions must be enumerated and tracked.
- Sources:
  - `PHASE2C_EXTRACTION_PACKET.md`

### FP-P2C-009 — BlockManager + storage invariants (deep scope, provisional until packet artifacts land)

- Legacy anchors:
  - `pandas/core/internals/managers.py` (`BaseBlockManager`, `BlockManager`, `SingleBlockManager`, `create_block_manager_from_blocks`, `_consolidate`)
- Behavioral invariants (provisional):
  - block placement and axis mappings are internally consistent (`blknos`/`blklocs`-class invariants),
  - consolidation rules preserve observable dtype/null semantics,
  - storage transforms do not silently corrupt downstream frame/index contracts.
- Hidden assumptions (provisional):
  - low-level storage invariants may require dedicated witness ledgers beyond high-level API fixture parity.
- Undefined-behavior edges (until extraction complete):
  - full BlockManager operation matrix and internals migration boundaries.
- Strict/hardened divergence:
  - strict: invariant breach is fail-closed,
  - hardened: bounded containment with mandatory forensic logging.
- Explicit non-goals (temporary):
  - none; deep-scope packet targets full scoped storage invariant parity.
- Sources:
  - `PHASE2C_EXTRACTION_PACKET.md`
  - `bd-2gi.24` design notes

### FP-P2C-010 — Full `loc`/`iloc` branch-path semantics (deep scope, provisional until packet artifacts land)

- Legacy anchors:
  - `pandas/core/indexing.py` (`_LocIndexer`, `_iLocIndexer`, `check_bool_indexer`, `convert_missing_indexer`)
- Behavioral invariants (provisional):
  - branch-path decisions are deterministic and mode-consistent,
  - boolean/indexer coercion semantics preserve pandas-observable contracts,
  - missing-label/indexer error classes remain explicit.
- Hidden assumptions (provisional):
  - coercion and indexer normalization logic spans multiple helper paths requiring branch matrix extraction.
- Undefined-behavior edges (until extraction complete):
  - full indexing branch matrix including advanced mixed indexer combinations.
- Strict/hardened divergence:
  - strict: fail-closed on unsupported branch surfaces,
  - hardened: allowlisted bounded continuation with decision ledger.
- Explicit non-goals (temporary):
  - none; deep-scope packet targets full scoped `loc`/`iloc` parity.
- Sources:
  - `PHASE2C_EXTRACTION_PACKET.md`
  - `bd-2gi.25` design notes

### FP-P2C-011 — Full GroupBy planner split/apply/combine + aggregate matrix (deep scope, provisional until packet artifacts land)

- Legacy anchors:
  - `pandas/core/groupby/grouper.py` (`Grouper`, `Grouping`, `get_grouper`)
  - `pandas/core/groupby/ops.py` planner/ops surface (`WrappedCythonOp`, `BaseGrouper`, `BinGrouper`, `DataSplitter`)
- Behavioral invariants (provisional):
  - planner decisions preserve grouping key determinism and output ordering contracts,
  - aggregate matrix preserves dtype/null semantics and default-option behavior,
  - split/apply/combine orchestration is reproducible under strict/hardened policy gates.
- Hidden assumptions (provisional):
  - multi-aggregate planner behavior has high interaction complexity and requires explicit witness tables.
- Undefined-behavior edges (until extraction complete):
  - full aggregate planner matrix and categorical/multi-key corner cases.
- Strict/hardened divergence:
  - strict: zero critical drift tolerated,
  - hardened: divergence only in explicit allowlisted defensive classes.
- Explicit non-goals (temporary):
  - none; deep-scope packet targets full scoped GroupBy planner parity.
- Sources:
  - `PHASE2C_EXTRACTION_PACKET.md`
  - `bd-2gi.26` design notes

## 4. Coverage Gaps (Actionable)

The following are required to fully satisfy `bd-2gi.1` and are currently incomplete:

1. FP-P2C-001..005 now include packet-local type/rule/error ledgers and invariant hook sections, but FP-P2C-006..011 still need equivalent extraction coverage.
2. Rule ledgers for 001..005 need branch-level predicate/default detail expansion to complete the full extraction payload contract depth.
3. Error ledgers for 001..005 need finalized exception message-class capture against pandas oracle text where scoped.
4. Invariant-to-counterexample/remediation linkage must be completed once packet differential mismatch corpora are generated for all active packets.
5. FP-P2C-006..011 now have provisional rows in this ledger; each row must be upgraded to fully extracted packet-local evidence once their artifact sets are produced.

## 5. Resolution Policy for Legacy Ambiguity

When legacy behavior is ambiguous or under-specified:

1. Prefer explicit pandas observable behavior from oracle fixtures.
2. If fixture evidence is absent, default strict mode to fail-closed.
3. Hardened mode may proceed only via explicit allowlisted defensive behavior.
4. Every ambiguity must record:
   - ambiguity class,
   - decision rationale,
   - strict/hardened outcome,
   - replay fixture ID and evidence pointer.

## 6. Implementation Guidance for Downstream Packets

- Packet-local files remain authoritative for implementation details.
- This foundation ledger is the cross-packet consistency layer and must be updated whenever packet contracts change.
- Any downstream packet marked complete without updating this ledger is considered incomplete for sign-off.
