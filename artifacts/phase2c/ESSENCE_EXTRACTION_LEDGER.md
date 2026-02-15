# ESSENCE_EXTRACTION_LEDGER.md — FrankenPandas Phase-2C Foundation

Date: 2026-02-14
Bead: `bd-2gi.1`
Status: in-progress foundation ledger (full coverage for FP-P2C-001..011; conformance harness pending for 006..011)

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

### FP-P2C-006 — Join + concat semantics

- Legacy anchors:
  - `pandas/core/reshape/merge.py` (`merge`, `_MergeOperation`, `get_join_indexers`)
  - `pandas/core/reshape/concat.py` (`concat`, `_Concatenator`, `_get_result`, `_make_concat_multiindex`)
  - `pandas/core/indexes/base.py` (`Index.join`, `Index.union`, `Index.intersection`)
- Behavioral invariants:
  - join cardinality matches key multiplicity semantics for each join mode (inner, left),
  - duplicate keys expand to deterministic cross-product cardinality with nested-loop ordering,
  - concat axis-0 preserves declared index ordering,
  - null-side handling is deterministic: left join inserts missing for unmatched right keys.
- Hidden assumptions:
  - scoped to inner and left join modes; right and outer deferred,
  - concat scoped to axis-0; axis-1 concat deferred,
  - multi-column merge keys not yet supported.
- Undefined-behavior edges:
  - full merge/concat option matrix (suffixes, indicator, validate),
  - right/outer join modes,
  - axis-1 concat with index alignment,
  - MultiIndex join/concat behavior.
- Strict/hardened divergence:
  - strict: fail-closed on unknown mode/metadata combinations,
  - hardened: bounded allowlisted defenses only.
- Explicit non-goals:
  - multi-column merges,
  - right/outer join modes (deferred),
  - axis-1 concat (deferred).
- Sources:
  - `artifacts/phase2c/FP-P2C-006/legacy_anchor_map.md`
  - `artifacts/phase2c/FP-P2C-006/contract_table.md`
  - `artifacts/phase2c/FP-P2C-006/risk_note.md`

### FP-P2C-007 — Missingness + nanops reductions

- Legacy anchors:
  - `pandas/core/missing.py` (`mask_missing`, `clean_fill_method`, `interpolate_2d_inplace`)
  - `pandas/core/nanops.py` (`nansum`, `nanmean`, `nanmedian`, `nanvar`, `nancorr`, `_ensure_numeric`)
  - `pandas/core/dtypes/missing.py` (`isna`, `notna`, `is_valid_na_for_dtype`)
- Behavioral invariants:
  - missing propagation is monotonic under composed operations (null in -> null out),
  - NaN/NaT/null distinctions are preserved at observable API boundaries,
  - reduction defaults and numeric coercion are deterministic (`skipna=True` default),
  - nansum of all-NaN returns 0.0; nanmean of all-NaN returns NaN,
  - fillna replaces missing values with cast scalar; dropna removes missing rows.
- Hidden assumptions:
  - NaN and Null both treated as missing in reductions (no NaN-vs-Null distinction in aggregation),
  - dtype-specific missing marker normalization is centralized in scalar/column contracts,
  - `skipna` parameter is always true (no `skipna=False` path implemented).
- Undefined-behavior edges:
  - full nanops option matrix (`skipna=False`, `min_count` parameter),
  - `interpolate` and `bfill`/`ffill` methods,
  - NaN-vs-NaT distinction in datetime operations.
- Strict/hardened divergence:
  - strict: fail-closed on unknown coercion/reduction ambiguity,
  - hardened: explicit bounded recovery only.
- Explicit non-goals:
  - `skipna=False` paths,
  - interpolation methods,
  - `min_count` parameter.
- Sources:
  - `artifacts/phase2c/FP-P2C-007/legacy_anchor_map.md`
  - `artifacts/phase2c/FP-P2C-007/contract_table.md`
  - `artifacts/phase2c/FP-P2C-007/risk_note.md`

### FP-P2C-008 — IO first-wave contract (CSV + JSON)

- Legacy anchors:
  - `pandas/io/parsers/readers.py` (`read_csv`, `TextFileReader`)
  - `pandas/io/parsers/c_parser_wrapper.py` (C-backed CSV parser)
  - `pandas/io/json/_json.py` (`read_json`, `to_json`, orient parameter dispatch)
  - `pandas/io/common.py` (IO path resolution, compression, encoding)
- Behavioral invariants:
  - CSV parser normalization is deterministic for scoped dialect (delimiter, header, index_col),
  - malformed input paths are fail-closed with deterministic diagnostics,
  - round-trip stability is preserved for supported schema/value surface,
  - JSON orient modes (records, columns, split, values, index) produce deterministic output,
  - dtype inference follows explicit priority: Int64 -> Float64 -> Utf8.
- Hidden assumptions:
  - first wave scoped to CSV and JSON formats only,
  - encoding is UTF-8 only; no codepage support,
  - compression not yet supported,
  - CSV parser is pure Rust (no C-backed parser).
- Undefined-behavior edges:
  - full CSV option matrix (quoting, escaping, encoding, thousands separator),
  - full JSON option matrix (date_format, default_handler, lines mode),
  - non-CSV/JSON IO formats (Excel, Parquet, SQL, HDF5),
  - chunked/streaming read for large files.
- Strict/hardened divergence:
  - strict: fail-closed on unsupported metadata/features,
  - hardened: bounded parser recovery for allowlisted corruption classes.
- Explicit non-goals:
  - non-CSV/JSON formats,
  - compression/decompression,
  - non-UTF-8 encoding,
  - chunked/streaming reads.
- Sources:
  - `artifacts/phase2c/FP-P2C-008/legacy_anchor_map.md`
  - `artifacts/phase2c/FP-P2C-008/contract_table.md`
  - `artifacts/phase2c/FP-P2C-008/risk_note.md`

### FP-P2C-009 — BlockManager + storage invariants (per-column model)

- Legacy anchors:
  - `pandas/core/internals/managers.py` (`BaseBlockManager`, `BlockManager`, `SingleBlockManager`, `create_block_manager_from_blocks`, `_consolidate`)
  - `pandas/core/internals/blocks.py` (`Block`, `DatetimeTZBlock`, `ExtensionBlock`)
  - `pandas/core/internals/construction.py` (`arrays_to_mgr`, `dict_to_mgr`)
- Behavioral invariants:
  - per-column storage with independently typed columns and packed bitvec validity masks,
  - ValidityMask length must equal values length (enforced at construction),
  - dtype is locked at construction and propagated deterministically through operations,
  - storage transforms do not silently corrupt downstream frame/index contracts.
- Hidden assumptions:
  - no BlockManager equivalent: FrankenPandas uses per-column storage, not blocked storage,
  - no consolidation pass: each column is independently typed and stored,
  - storage invariants are simpler than pandas due to per-column model.
- Undefined-behavior edges:
  - full BlockManager operation matrix and internals migration boundaries,
  - in-place mutation semantics (pandas copy-on-write vs immutable columns),
  - memory layout optimization (Arrow-compatible, SIMD alignment),
  - block consolidation and deconsolidation paths.
- Strict/hardened divergence:
  - strict: invariant breach is fail-closed,
  - hardened: bounded containment with mandatory forensic logging.
- Explicit non-goals:
  - BlockManager consolidation,
  - in-place mutation,
  - Arrow-compatible memory layouts.
- Sources:
  - `artifacts/phase2c/FP-P2C-009/legacy_anchor_map.md`
  - `artifacts/phase2c/FP-P2C-009/contract_table.md`
  - `artifacts/phase2c/FP-P2C-009/risk_note.md`
  - `bd-2gi.24` design notes

### FP-P2C-010 — Full `loc`/`iloc` branch-path semantics

- Legacy anchors:
  - `pandas/core/indexing.py` (`_LocIndexer`, `_iLocIndexer`, `check_bool_indexer`, `convert_missing_indexer`)
  - `pandas/core/indexes/base.py` (`Index.get_loc`, `Index.get_indexer`, `Index.slice_locs`)
  - `pandas/core/series.py` (`__getitem__`, `__setitem__`, `_get_with`)
- Behavioral invariants:
  - boolean mask indexing treats null values as false (not selected),
  - mask length must match target length exactly,
  - filter preserves selected labels in original order,
  - head/tail provide positional slicing with bounds clamping.
- Hidden assumptions:
  - full `loc`/`iloc` API not yet implemented; current coverage is boolean mask filtering and head/tail,
  - scalar label lookup, list-of-labels selection, and label-slice selection are deferred,
  - coercion and indexer normalization logic spans multiple helper paths requiring branch matrix extraction.
- Undefined-behavior edges:
  - full `loc` branch matrix (scalar, list, slice, callable, boolean),
  - full `iloc` branch matrix (scalar, list, slice, boolean),
  - mixed indexer combinations,
  - `__setitem__` assignment paths,
  - advanced boolean indexer with MultiIndex.
- Strict/hardened divergence:
  - strict: fail-closed on unsupported branch surfaces,
  - hardened: allowlisted bounded continuation with decision ledger.
- Explicit non-goals:
  - full loc/iloc API beyond boolean mask and positional slicing,
  - setitem assignment paths,
  - MultiIndex indexing.
- Sources:
  - `artifacts/phase2c/FP-P2C-010/legacy_anchor_map.md`
  - `artifacts/phase2c/FP-P2C-010/contract_table.md`
  - `artifacts/phase2c/FP-P2C-010/risk_note.md`
  - `bd-2gi.25` design notes

### FP-P2C-011 — Full GroupBy planner split/apply/combine + aggregate matrix

- Legacy anchors:
  - `pandas/core/groupby/grouper.py` (`Grouper`, `Grouping`, `get_grouper`)
  - `pandas/core/groupby/ops.py` (`WrappedCythonOp`, `BaseGrouper`, `BinGrouper`, `DataSplitter`)
  - `pandas/core/groupby/groupby.py` (`GroupBy`, `SeriesGroupBy`, `DataFrameGroupBy`)
  - `pandas/core/groupby/generic.py` (aggregation dispatch)
- Behavioral invariants:
  - first-seen key encounter order defines output ordering when `sort=False`,
  - aggregate matrix preserves dtype/null semantics (Sum, Mean, Count, Min, Max, First, Last, Std, Var, Median),
  - `dropna=True` excludes null keys from grouping,
  - each aggregate function has defined semantics for empty and all-null groups,
  - dense Int64 fast path and arena-backed path produce identical output to generic path.
- Hidden assumptions:
  - single-key series groupby only; multi-key DataFrame groupby deferred,
  - no `transform`, `filter`, or `apply` paths implemented,
  - no `observed` parameter for categorical key handling.
- Undefined-behavior edges:
  - full aggregate planner matrix (custom functions, named aggregation),
  - multi-key DataFrame groupby semantics,
  - categorical key handling with `observed` parameter,
  - `transform`, `filter`, `apply` paths,
  - rolling/expanding window aggregation.
- Strict/hardened divergence:
  - strict: zero critical drift tolerated,
  - hardened: divergence only in explicit allowlisted defensive classes.
- Explicit non-goals:
  - multi-key DataFrame groupby,
  - transform/filter/apply paths,
  - rolling/expanding window aggregation,
  - custom aggregation functions.
- Sources:
  - `artifacts/phase2c/FP-P2C-011/legacy_anchor_map.md`
  - `artifacts/phase2c/FP-P2C-011/contract_table.md`
  - `artifacts/phase2c/FP-P2C-011/risk_note.md`
  - `bd-2gi.26` design notes

## 4. Coverage Gaps (Actionable)

The following are required to fully satisfy `bd-2gi.1` and are currently incomplete:

1. ~~FP-P2C-006..011 need equivalent extraction coverage.~~ DONE: All packets now have legacy_anchor_map.md, contract_table.md, and risk_note.md.
2. ~~Rule ledgers for 001..005 need branch-level predicate/default detail expansion to complete the full extraction payload contract depth.~~ DONE: All 5 packets deepened with branch-level predicates, exact line references, edge cases, and full error ledger entries with pandas-equivalent error strings.
3. Error ledgers for 001..005 need finalized exception message-class capture against pandas oracle text where scoped.
4. Invariant-to-counterexample/remediation linkage must be completed once packet differential mismatch corpora are generated for all active packets.
5. ~~FP-P2C-006..011 provisional rows must be upgraded.~~ DONE: All provisional entries upgraded to extracted entries with packet-local artifact sources.
6. ~~FP-P2C-006..011 need conformance harness integration (fixture_manifest.json, parity_gate.yaml, parity artifacts).~~ DONE: All packets 006..011 now have fixture_manifest.json, parity_gate.yaml, and conformance fixture files. All 29 fixtures pass (446 workspace tests green).

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
