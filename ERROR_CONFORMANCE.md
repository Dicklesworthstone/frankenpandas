# Error-Path Conformance Tracker

Per br-frankenpandas-niwb: pandas exception-category parity is currently
tested by ~96 of 1,249 fixtures (~7.7%). The forward harness compares
error *messages* via `expected_error_contains`; it does not check that we
raise the same *category* of exception pandas would (TypeError vs
ValueError vs KeyError vs ...).

This document tracks the per-op-class gap and the path to lifting coverage.

## Categories

The comparator groups pandas exceptions into six stable buckets. A
frankenpandas `FrameError` variant maps to at most one bucket.

| Category       | Pandas exception types                              | Our `FrameError` variants                                                                         |
| -------------- | --------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| `Type`         | `TypeError`                                         | `DTypeMismatch`, `UnsupportedOperation`, `IncompatibleDType`                                      |
| `Value`        | `ValueError`                                        | `InvalidArgument`, `ShapeMismatch`, `ParseError`                                                  |
| `Key`          | `KeyError`                                          | `ColumnNotFound`, `IndexLabelNotFound`                                                            |
| `Index`        | `IndexError`                                        | `OutOfBounds`, `DuplicateLabel`                                                                   |
| `IO`           | `IOError`, `OSError`, `FileNotFoundError`           | `FpIoError::*`                                                                                    |
| `Unsupported`  | `NotImplementedError`                               | `UnsupportedOperation` (when routed from compat-closure rejection)                                |

Many-to-one mappings (e.g. `UnsupportedOperation` appearing in both
`Type` and `Unsupported`) are resolved by inspecting the context in the
compat-closure layer before reporting; edge cases are tracked under
DISCREPANCIES.md.

## Current coverage (baseline — 2026-04-23)

Grep of `fixtures/packets/*.json` for `expected_error`:

- Total fixtures carrying `expected_error`: **96 / 1,249** (7.7%).
- Fixtures with `expected_error_category` field: **0** (new field; this
  bead Phase 2 populates it).

Per op-class breakdown (top 10 by packet count):

| Op class                       | Positive fixtures | Error fixtures | Category-tagged |
| ------------------------------ | ----------------: | -------------: | --------------: |
| `series_add` / `series_sub`    | ~180              | ~20            |               0 |
| `dataframe_groupby_*`          | ~220              | ~15            |               0 |
| `dataframe_merge_*`            | ~95               | ~18            |               0 |
| `dataframe_query`              | ~30               | ~8             |               0 |
| `series_str_*`                 | ~90               | ~5             |               0 |
| `io_read_csv` / `io_read_json` | ~110              | ~12            |               0 |
| (remaining ~45 op classes)     | (long tail)       | ~18            |               0 |

## Action plan

Phase 1 (this commit — scaffolding):
- Create this tracker.
- Reserve `expected_error_category` field name in `PacketFixture`
  (future struct-field addition, absent today).

Phase 2 (future slice):
- Add `ExpectedErrorCategory` enum + serde field to `PacketFixture`.
- Add `compare_expected_error_category` comparator in `fp-conformance`.
- Backfill `expected_error_category` on the existing 96 error fixtures.

Phase 3 (future slice):
- For each op class in the table above, synthesize 2-4 additional
  error fixtures (wrong dtype, missing column, empty input, negative
  integer where non-negative required, division by zero, etc.).
- Lift error-path coverage from 7.7% → target 30% by the end of
  Phase 3, 50%+ in a follow-up sweep.

Phase 4 (future slice):
- DISCREPANCIES.md entries for any category pair that cannot be
  faithfully translated (e.g. pandas raising a `pandas.errors.*` that
  doesn't match any of our categories cleanly).

## Related beads

- br-frankenpandas-urhy: oracle pytest suite (landed 2026-04-23). Some
  of the "exception-capture" helpers land there first.
- br-frankenpandas-nato: RequirementLevel tagging on fixtures. An
  error-path fixture with `category=Type, level=Must` is the unit that
  drives the compliance matrix.
- br-frankenpandas-2ssw: Categorical has 1 fixture for ~50 methods;
  error-path for Categorical is a subset of this bead's work.
