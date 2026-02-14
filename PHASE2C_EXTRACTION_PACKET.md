# PHASE2C_EXTRACTION_PACKET.md — FrankenPandas

Date: 2026-02-13

Purpose: convert Phase-2 analysis into direct implementation tickets with concrete legacy anchors, target crates, and oracle tests.

## 1. Ticket Packets

| Ticket ID | Subsystem | Legacy anchors (classes/functions) | Target crates | Oracle tests |
|---|---|---|---|---|
| `FP-P2C-001` | DataFrame/Series construction + alignment | `DataFrame`, `_from_nested_dict`, `_reindex_for_setitem` in `pandas/core/frame.py`; `Series` in `pandas/core/series.py` | `fp-frame`, `fp-types`, `fp-columnar` | `pandas/tests/frame/test_constructors.py`, `pandas/tests/series/test_constructors.py` |
| `FP-P2C-002` | Index model and indexer semantics | `Index`, `ensure_index`, `_validate_join_method` in `pandas/core/indexes/base.py` | `fp-index`, `fp-types` | `pandas/tests/indexes/test_base.py`, `pandas/tests/indexes/test_indexing.py` |
| `FP-P2C-003` | BlockManager and storage invariants | `BaseBlockManager`, `BlockManager`, `SingleBlockManager`, `create_block_manager_from_blocks`, `_consolidate` in `pandas/core/internals/managers.py` | `fp-columnar` | `pandas/tests/frame/*`, `pandas/tests/internals/*` |
| `FP-P2C-004` | `loc`/`iloc` behavior | `_LocIndexer`, `_iLocIndexer`, `check_bool_indexer`, `convert_missing_indexer` in `pandas/core/indexing.py` | `fp-frame`, `fp-index` | `pandas/tests/indexing/*` |
| `FP-P2C-005` | Groupby planner and split/apply/combine | `Grouper`, `Grouping`, `get_grouper` in `core/groupby/grouper.py`; `WrappedCythonOp`, `BaseGrouper`, `BinGrouper`, `DataSplitter` in `core/groupby/ops.py` | `fp-groupby`, `fp-expr` | `pandas/tests/groupby/*` |
| `FP-P2C-006` | Join + concat semantics | `merge`, `_MergeOperation`, `get_join_indexers` in `core/reshape/merge.py`; `concat`, `_get_result`, `_make_concat_multiindex` in `core/reshape/concat.py` | `fp-join`, `fp-frame` | `pandas/tests/reshape/*`, `pandas/tests/reshape/merge/*` |
| `FP-P2C-007` | Missingness and nanops reductions | `mask_missing`, `clean_fill_method`, `interpolate_2d_inplace` in `core/missing.py`; `nansum`, `nanmean`, `nanmedian`, `nanvar`, `nancorr` in `core/nanops.py` | `fp-expr`, `fp-types` | `pandas/tests/arrays/*`, `pandas/tests/scalar/*`, `pandas/tests/window/*` |
| `FP-P2C-008` | IO first-wave contract | parser and formatter entrypoints in `pandas/io/*` (scoped to CSV + schema normalization) | `fp-io` | `pandas/tests/io/formats/*`, `pandas/tests/io/parser/*` |

## 2. Packet Definition Template

For each ticket above, deliver all of the following artifacts in the same PR:

1. `legacy_anchor_map.md`: exact source path + line anchors + extracted behavior notes.
2. `contract_table.md`: input contract, output contract, error contract, null/index behavior.
3. `fixture_manifest.json`: mapped oracle tests and normalized fixture IDs.
4. `parity_gate.yaml`: pass/fail thresholds for strict + hardened mode.
5. `risk_note.md`: security and compatibility risks introduced/covered.

## 3. Strict/Hardened Expectations per Packet

- Strict mode: exact scoped pandas-observable behavior.
- Hardened mode: same outward contract with bounded defensive checks.
- Unknown incompatible metadata/path: fail-closed in both modes.

## 4. Immediate Execution Order

1. `FP-P2C-001`
2. `FP-P2C-002`
3. `FP-P2C-003`
4. `FP-P2C-004`
5. `FP-P2C-006`
6. `FP-P2C-005`
7. `FP-P2C-007`
8. `FP-P2C-008`

## 5. Done Criteria (Phase-2C)

- All 8 packets have extracted anchor maps and contract tables.
- At least one runnable fixture family exists per packet in `fp-conformance`.
- Packet-level parity report schema is produced for every packet.
- RaptorQ sidecars are generated for fixture bundles and parity reports.

## 6. Per-Ticket Extraction Schema (Mandatory Fields)

Every `FP-P2C-*` packet deliverable MUST include these normalized fields:
1. `packet_id`
2. `legacy_paths`
3. `legacy_symbols`
4. `input_contract`
5. `output_contract`
6. `error_contract`
7. `null_contract`
8. `index_alignment_contract`
9. `strict_mode_policy`
10. `hardened_mode_policy`
11. `excluded_scope`
12. `oracle_tests`
13. `performance_sentinels`
14. `compatibility_risks`
15. `raptorq_artifacts`

If any field is missing, the packet is automatically `NOT READY`.

## 7. Risk Tiering and Gate Escalation

| Ticket | Risk tier | Why | Extra gate |
|---|---|---|---|
| `FP-P2C-001` | Critical | constructor + alignment semantics define most downstream behavior | double differential replay |
| `FP-P2C-003` | Critical | BlockManager invariants affect storage correctness | invariant proof ledger |
| `FP-P2C-004` | High | indexing semantics have high user-visible drift risk | branch-path fixture minimum |
| `FP-P2C-005` | High | groupby defaults/order frequently regress | ordering witness checks |
| `FP-P2C-006` | Critical | join cardinality drift is catastrophic | cardinality witness suite |
| `FP-P2C-007` | High | null propagation regressions are subtle and frequent | null truth-table lock |
| `FP-P2C-008` | Medium | IO breadth large but staged | parser adversarial minimum |

Gate law:
- Critical packets require strict drift `0` and hardened divergence only in allowlisted defensive categories.

## 8. Packet Artifact Topology (Normative)

Directory template for each packet:
- `artifacts/phase2c/ESSENCE_EXTRACTION_LEDGER.md` (cross-packet canonical foundation ledger; must be updated whenever packet essence changes)
- `artifacts/phase2c/FP-P2C-00X/legacy_anchor_map.md`
- `artifacts/phase2c/FP-P2C-00X/contract_table.md`
- `artifacts/phase2c/FP-P2C-00X/fixture_manifest.json`
- `artifacts/phase2c/FP-P2C-00X/parity_gate.yaml`
- `artifacts/phase2c/FP-P2C-00X/risk_note.md`
- `artifacts/phase2c/FP-P2C-00X/parity_report.json`
- `artifacts/phase2c/FP-P2C-00X/parity_report.raptorq.json`
- `artifacts/phase2c/FP-P2C-00X/parity_report.decode_proof.json`

## 9. Optimization and Isomorphism Proof Hooks

For each packet, optimization is allowed only after first strict parity pass.

Required proof block in `risk_note.md`:
- ordering preserved: yes/no + justification
- tie-breaking preserved: yes/no + justification
- null/NaN/NaT behavior preserved: yes/no + justification
- fixture checksum verification: pass/fail

## 10. Packet Readiness Rubric

A packet is `READY_FOR_IMPL` only when all conditions hold:
1. extraction schema fields complete,
2. fixture manifest covers at least one happy path + one edge path + one adversarial path,
3. strict/hardened parity gates are defined and machine-checkable,
4. risk note has explicit compatibility and security mitigations,
5. RaptorQ sidecar and decode proof are generated for parity report.
