# Join and Merge Null-Key Metamorphic Matrix - 2026-05-08

Bead: `br-frankenpandas-tn6qb.8`

Scope: strict-mode pandas-observable merge behavior around Null/NaN keys.
Index-based joins do not currently have a Null/NaN label representation, so the
implemented slice targets DataFrame column-key merge semantics in `fp-join`.
Hardened-mode recovery behavior is out of scope.

## Property Matrix

| MR | API | Transformation | Invariant | Covers | Score |
| --- | --- | --- | --- | --- | --- |
| MR-01 | `merge_dataframes(..., Inner)` | Add left-only and right-only non-missing key rows to frames that already contain duplicate Null/NaN keys | Inner output for existing matched missing and non-missing keys is unchanged | Null, NaN, duplicate missing keys, unmatched non-missing labels | 4.5 |
| MR-02 | `merge_dataframes_on_with_options(..., Inner/Outer)` | Compare sorted `inner` merge to `outer` merge filtered by indicator value `both` | `outer[outer._merge == "both"]` has the same value projection as sorted `inner` | mixed Int64/Float64 key promotion, Null/NaN key matching, left-only/right-only rows | 4.4 |
| MR-03 | Index join | Join on actual Null/NaN index labels | Deferred until `IndexLabel` can represent missing labels directly | missing index labels | Deferred |

## Implemented Tests

- `merge_null_nan_keys_metamorphic_unmatched_rows_do_not_disturb_inner_tn6qb8`
- `merge_mixed_dtype_missing_keys_metamorphic_outer_both_equals_inner_tn6qb8`

## Pandas Oracle Snapshot

Lightweight pandas check on 2026-05-08 confirmed that `pd.merge` matches rows
whose merge keys are both null-like, producing a Cartesian product across
missing-key duplicates. The implemented tests pin that relation without copying
pandas internals.

## Mode Boundary

These tests assert strict pandas parity. Hardened-mode behavior must use
separate policy setup and separate test names before it can change merge
admission, missing-key matching, or recovery behavior.
