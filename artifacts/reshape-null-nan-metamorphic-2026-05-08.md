# Reshape Null/NaN Metamorphic Matrix - 2026-05-08

Bead: `br-frankenpandas-tn6qb.9`

Scope: strict-mode pandas-observable reshape behavior around Null/NaN values,
missing keys, duplicate labels, signed zero, and concat keys. Hardened-mode
recovery is out of scope.

## Property Matrix

| MR | API | Transformation | Invariant | Covers | Score |
| --- | --- | --- | --- | --- | --- |
| MR-01 | `DataFrame::melt` | Omit `value_vars` when all non-id columns are the same explicit value columns | Automatic and explicit melts have identical column order and value projection | Null/NaN id values, mixed value dtype promotion, signed zero | 4.3 |
| MR-02 | `DataFrame::pivot_table` | Add rows whose row or column grouping key is Null/NaN under default `dropna=true` | Existing complete-key pivot output is unchanged | missing grouping keys, duplicate complete keys, default strict dropna behavior | 4.6 |
| MR-03 | `concat_dataframes_with_keys` | Add concat keys to row-wise concat inputs | Value projection is identical to plain concat; only index labels are key-prefixed | concat keys, duplicate index labels, Null/NaN payloads | 4.1 |

## Implemented Tests

- `reshape_null_nan_metamorphic_melt_auto_matches_explicit_tn6qb9`
- `reshape_null_nan_metamorphic_pivot_table_missing_keys_do_not_disturb_default_tn6qb9`
- `reshape_null_nan_metamorphic_concat_keys_preserve_value_projection_tn6qb9`
- `conformance_reshape_melt_null_nan_ids_auto_value_vars_tn6qb9`
- `conformance_reshape_pivot_table_missing_keys_dropna_default_tn6qb9`

## Pandas Oracle Snapshot

Lightweight pandas checks on 2026-05-08 confirmed:

- `pd.melt(..., id_vars=["id"])` with omitted `value_vars` uses all non-id
  columns and preserves Null/NaN id rows, mixed numeric values, and signed zero
  value payloads in value-variable-major order.
- `pd.pivot_table(..., dropna=True, sort=False)` drops rows whose grouping
  keys are null-like, so adding Null/NaN-key rows does not disturb complete-key
  cells.
- `pd.concat(..., keys=[...])` changes the row index to a hierarchical key
  projection but preserves the row-wise value projection, including duplicate
  labels and null-like payloads.

## Mode Boundary

These tests pin strict pandas parity. Any hardened-mode recovery for malformed
reshape inputs must live under separate policy setup and separate test names.
