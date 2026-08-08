# Fixture divergence triage — 2026-08-08 (BlueRobin)

Bead: `br-frankenpandas-fixture-divergence-triage-9s0c4`
Companion: `br-frankenpandas-fixture-corpus-stale-vs-oracle-p6srr` (the same rows are its red residue)

Corpus state at time of triage: freshness gate **235** red; **151** fixtures MOVED and unattributed.
Reproduce with `python3 scripts/regenerate_fixtures.py --jobs 8 --report-json <path>` and read
`moved_unattributed`.

## Result

**One mechanism accounts for 81 of the 151 (53.6%).**

| rows | share | mechanism |
|-----:|------:|-----------|
| 81 | 53.6% | `MISSING-KIND-NOT-DTYPE-DERIVED` — see below |
| 11 | 7.3% | `KIND int64->float64` alone (promotion without a visible marker change) |
| 59 | 39.1% | residue, needs individual attention |

The 81 are exactly the fixtures whose only classes are `NULL_MARKER null->na_n`, optionally with
`KIND int64->float64`. They split 48 round-trip (the fixture's own input carried the marker) and
33 op-introduced (the operation created the missing value) — but see below: the split does **not**
change the mechanism.

## The mechanism

> **CORRECTED after first publication.** The original wording of this section said the missing
> kind is "a property of the COLUMN DTYPE" and that object columns therefore keep `None`. That is
> true only for missing values the CALLER SUPPLIED. It is wrong for missing values pandas
> INTRODUCES, and implementing from it would make object-column reindex/merge insert `None`, which
> pandas never does. The corrected rule is below; the 81-row attribution is unchanged.

There are **two** rules, and only the first one governs these 81 fixtures.

**Rule 1 — an INTRODUCED missing value is always a float `nan`, in every dtype.** Not dtype-derived
at all. Measured on live pandas 2.2.3, taking the element that the operation invented:

```
pd.Series(['a','b']).reindex([0,1,2])      dtype=object   introduced=float nan
pd.Series([True,False]).reindex([0,1,2])   dtype=object   introduced=float nan
pd.Series([1,2]).reindex([0,1,2])          dtype=float64  introduced=float nan
pd.Series(['a',1]).reindex([0,1,2])        dtype=object   introduced=float nan
df.merge(..., how='left')  missing 't'     dtype=object   ['str', 'float']
pd.concat([...], axis=0)   missing 's'     dtype=object   ['str', 'float']
```

An OBJECT column gets `nan`, not `None`. That is the part the first draft got backwards.

**Rule 2 — a SUPPLIED `None` survives only if the dtype can store it.** This is the dtype-derived
half, and it explains the round-trip rows:

```
object   -> ['str', 'NoneType', 'str']     None preserved (object can hold it)
float64  -> ['float', 'float', 'float']    became NaN
Int64    -> ['int', 'NAType', 'int']       became pd.NA
datetime64[ns] / timedelta64[ns]           became NaT
```

FrankenPandas violates Rule 1 everywhere — it introduces `NullKind::Null` — and violates Rule 2 for
every non-object dtype. Both produce the same observable, `NULL_MARKER null->na_n`, which is why the
48 round-trip and 33 op-introduced rows land in one class.

The `KIND int64->float64` half of the signature is Rule 1's corollary: numpy int64 cannot hold a
`nan`, so introducing one forces float64.

Corpus confirmation of the object case, not just the synthetic probe —
`fp_p2d_031_dataframe_concat_axis0_outer_utf8_overlap_strict` has a `city` column of kinds
`['utf8', 'null']` whose null is pinned `null` while the oracle emits `na_n`. A string column, an
introduced miss, and pandas still says NaN.

```python
pd.DataFrame.from_records([{'a':1},{'a':2}], columns=['a','z'])
#    a    z          <- absent column 'z' is an all-NaN FLOAT64 column
# 0  1  NaN
# dtypes: a int64, z float64
```

⚠️ Do NOT generalise Rule 1 to "all-missing columns are float64". Measured:
`pd.Series([None, None])` and `pd.DataFrame({'z': [None, None]})` are both **object** — that is
SUPPLIED data under Rule 2. Only the column pandas invents (`from_records` with an absent name) is
float64.

Concrete corpus example, `fp_p2d_018_dataframe_from_records_column_order_new_column_null_hardened`:

```
PINNED  z: [{"kind":"null","value":"null"}, {"kind":"null","value":"null"}]
ORACLE  z: [{"kind":"null","value":"na_n"}, {"kind":"null","value":"na_n"}]
```

## This does NOT contradict lufpu / joeff

`br-frankenpandas-str-null-kind-identity-lufpu` fixed string ops to PRESERVE `None`, and
`br-frankenpandas-joeff` deliberately made null kinds distinct with kind-sensitive `Eq`/`Hash`.
Both are correct and remain correct, and the corrected rules above make the boundary sharper rather
than blurrier: lufpu is **Rule 2** on an object column — a `None` the caller supplied, carried
through a kernel that must not rewrite it. These 81 rows are almost all **Rule 1** — a missing value
the OPERATION invented, which pandas makes `nan` even in an object column. Different paths, opposite
obligations, no conflict. Preserving a supplied `None` and minting `nan` for an invented gap are both
required.

lufpu's own closeout already found the dtype-dependence from the other side — it had to route six
int-returning accessors through `apply_str_missing_nan` because "pandas splits by RESULT DTYPE, not
accessor family". Same rule, found twice from opposite directions.

## Residue (59 rows) — not attributed here

| rows | classes |
|-----:|---------|
| 18 | `VALUE` |
| 5 | `ERROR expected-but-succeeded` |
| 4 | `NULL_MARKER na_n->null` (the OPPOSITE direction — likely a distinct mechanism) |
| 4 | `KIND float64->int64` |
| 3 | `KIND utf8->int64 + VALUE` |
| 3 | `KIND null->utf8 + VALUE` |
| 3 | `SHAPE longer` |
| 2 each | several mixed signatures |
| 1 each | 10 singleton signatures |

The 4 `na_n->null` rows deserve their own look precisely because they run the other way: the oracle
produces `null` where the fixture pins `na_n`, which the rule above does not explain.

## What must NOT be done with this

These 81 rows are **not** cleared for regeneration by this document. Naming the mechanism is not the
same as deciding the remedy, and the remedy is a real FrankenPandas change — mint `NullKind::NaN`
wherever an operation INTRODUCES a missing value (Rule 1), normalise a supplied `None` to the
dtype's storable missing (Rule 2, which `Scalar::missing_for_dtype` already tabulates correctly),
and promote int64 -> float64 on introduction. Not a fixture rewrite. Regenerating them first would
bank pandas' answer while FP still produces the old one, turning 81 currently-honest red rows into
81 green lies. Filed as its own bead.

**Where the machinery already is.** `Scalar::missing_for_dtype` (fp-types) is already correct:
Float64 -> `Null(NaN)`, Datetime64/Timedelta64 -> `NAT`, Utf8/Int64 -> `Null(Null)`. And
`Column::normalize_missing_for_dtype` (fp-columnar) already applies it, passing NaN/NaT through
unchanged. So Rule 2 is mostly present. What is missing is Rule 1: the fill sites that invent a
missing value hand it `NullKind::Null` instead of `NullKind::NaN`, and there is no int64 -> float64
promotion on introduction. Start by finding the null-fill sites, not by rewriting the kind table.

## Ownership: the 81 have FOUR owners, not one

> **SECOND CORRECTION.** The single-mechanism framing above is right about the *observable*
> (`NULL_MARKER null->na_n`) but wrong about the *owner*. Attempting to implement `nywa8` across all
> 81 would have double-fixed rows belonging to another bead and "fixed" FrankenPandas where it is
> already correct. Split, with the class data:

| rows | owner | what it is |
|-----:|-------|------------|
| 32 | `nywa8` | structural, promotion + marker — `dataframe_concat` (19), `merge` (3), `constructor_kwargs` (3), `constructor_list_like` (3), `sort_values` (2), `from_records` (1), `series_tail` (1) |
| 22 | `nywa8` | structural, marker only — `dict_of_series` (3), `constructor_kwargs` (2), `astype` (2), and singletons incl. rolling/expanding |
| 11 | **`lwvet`** | accessor int->float64 promotion + marker (`str_count_*`, `str_split_count`, `dt_days_in_month`, …) — ALREADY FILED as "int-returning str accessors never promote to Float64" |
| 16 | mixed | accessor, marker only — **needs per-op verification, see below** |

**The 54 structural rows are the confirmed `nywa8` core** and the only subset cleared for
implementation. Direct pandas measurement backs every one of their mechanisms (reindex, merge,
concat, `from_records` all mint `nan`).

### The 16 accessor rows are NOT uniformly FP's fault

Verified per namespace, live pandas 2.2.3 on `pd.Series(['Hello', None, 'WORLD'], dtype=object)`:

```
str.lower / strip / contains / startswith / findall  -> ['...', 'NoneType', '...']   None PRESERVED
dt.strftime / dt.month_name (on a NaT input)         -> ['str', 'float']             nan
dt.days_in_month                                     -> float64 nan
```

So `str.*` on object input **preserves None** — lufpu was right, including for the bool-returning
family — while `dt.*` maps a NaT input to `nan` in the derived output. The dt rows are genuine; the
str rows mostly are not.

Three are outright **oracle defects, with FrankenPandas correct**: `contains_any`, `startswith_any`,
`endswith_any` go through `_str_any_op`, which does `.fillna(False)` per pattern and then restores
missingness with `.where(series.notna())` — and `Series.where` inserts **NaN**, which cannot restore
a `None`. Measured:

```
s.str.contains('oo', regex=False)                    -> ['bool', 'NoneType', 'bool', 'bool']
same .fillna(False).where(s.notna())                 -> ['bool', 'float',    'bool', 'bool']
```

Filed as `br-frankenpandas-6e6ag`. **Do not change FP for these three.**

`index_of` / `rindex_of` are a third case again: `_index_of_result` builds nan-on-absent
*deliberately*, documented as modelling FP's own nullable-int contract for ops pandas has no
equivalent of. Read it before touching it.

## The 81 fixtures

Case ids, sorted. These are the rows `br-frankenpandas-nywa8` must fix in FrankenPandas
BEFORE any of them is regenerated.

```
fp_p2c_010_series_head_with_nulls_hardened
fp_p2d_014_dataframe_concat_column_mismatch_error_strict
fp_p2d_014_dataframe_merge_column_left_missing_hardened
fp_p2d_014_dataframe_merge_column_right_order_strict
fp_p2d_017_dataframe_from_series_nullable_float_alignment_hardened
fp_p2d_018_dataframe_from_records_column_order_new_column_null_hardened
fp_p2d_018_dataframe_from_records_sparse_keys_null_fill_hardened
fp_p2d_019_dataframe_constructor_kwargs_column_missing_null_hardened
fp_p2d_019_dataframe_constructor_kwargs_empty_frame_with_index_columns_strict
fp_p2d_019_dataframe_constructor_kwargs_row_and_column_projection_hardened
fp_p2d_019_dataframe_constructor_kwargs_row_reindex_with_missing_strict
fp_p2d_019_dataframe_constructor_kwargs_utf8_index_reindex_strict
fp_p2d_020_dataframe_constructor_dict_of_series_explicit_index_reorder_hardened
fp_p2d_020_dataframe_constructor_dict_of_series_row_and_column_projection_strict
fp_p2d_020_dataframe_constructor_dict_of_series_union_align_strict
fp_p2d_020_dataframe_constructor_scalar_null_broadcast_strict
fp_p2d_021_dataframe_constructor_list_like_short_rows_null_fill_strict
fp_p2d_022_dataframe_constructor_list_like_null_fill_determinism_hardened
fp_p2d_022_dataframe_constructor_list_like_ragged_three_rows_null_padding_hardened
fp_p2d_028_dataframe_concat_axis1_basic_strict
fp_p2d_028_dataframe_concat_axis1_order_left_then_unseen_hardened
fp_p2d_028_dataframe_concat_axis1_preserves_existing_nulls_hardened
fp_p2d_028_dataframe_concat_axis1_sparse_alignment_strict
fp_p2d_028_dataframe_concat_axis1_utf8_index_strict
fp_p2d_031_dataframe_concat_axis0_outer_basic_overlap_strict
fp_p2d_031_dataframe_concat_axis0_outer_disjoint_columns_hardened
fp_p2d_031_dataframe_concat_axis0_outer_duplicate_index_hardened
fp_p2d_031_dataframe_concat_axis0_outer_empty_right_schema_strict
fp_p2d_031_dataframe_concat_axis0_outer_preserves_nulls_strict
fp_p2d_031_dataframe_concat_axis0_outer_utf8_overlap_strict
fp_p2d_032_dataframe_concat_axis0_outer_basic_column_order_strict
fp_p2d_032_dataframe_concat_axis0_outer_disjoint_column_order_hardened
fp_p2d_032_dataframe_concat_axis0_outer_duplicate_index_order_hardened
fp_p2d_032_dataframe_concat_axis0_outer_empty_left_schema_order_hardened
fp_p2d_032_dataframe_concat_axis0_outer_empty_right_schema_order_strict
fp_p2d_032_dataframe_concat_axis0_outer_preserves_nulls_order_strict
fp_p2d_032_dataframe_concat_axis0_outer_utf8_column_order_strict
fp_p2d_033_dataframe_merge_composite_left_right_alias_outer_hardened
fp_p2d_037_dataframe_merge_sort_true_right_hardened
fp_p2d_043_series_sort_values_numeric_ascending_na_last_strict
fp_p2d_043_series_sort_values_numeric_descending_na_last_hardened
fp_p2d_044_series_tail_negative_preserves_nulls_hardened
fp_p2d_288_series_str_findall_null_hardened
fp_p2d_289_series_str_contains_any_null_hardened
fp_p2d_290_series_str_startswith_any_empty_patterns_hardened
fp_p2d_291_series_str_endswith_any_null_hardened
fp_p2d_292_series_str_split_count_null_hardened
fp_p2d_293_series_str_split_get_oob_hardened
fp_p2d_294_series_str_rsplit_get_null_hardened
fp_p2d_296_series_str_count_matches_null_hardened
fp_p2d_297_series_str_count_literal_null_hardened
fp_p2d_299_series_str_index_of_null_hardened
fp_p2d_300_series_str_rindex_of_null_hardened
fp_p2d_310_series_dt_strftime_null_hardened
fp_p2d_321_series_dt_days_in_month_null_hardened
fp_p2d_323_series_dt_dayofyear_null_hardened
fp_p2d_325_series_dt_year_null_hardened
fp_p2d_326_series_dt_hour_null_hardened
fp_p2d_327_series_dt_minute_null_hardened
fp_p2d_328_series_dt_second_null_hardened
fp_p2d_329_series_dt_dayofweek_null_hardened
fp_p2d_330_series_dt_quarter_null_hardened
fp_p2d_331_series_dt_month_name_null_hardened
fp_p2d_332_series_dt_day_name_null_hardened
fp_p2d_333_series_dt_total_seconds_null_hardened
fp_p2d_334_series_dt_to_timestamp_null_hardened
fp_p2d_336_series_rolling_std_null_hardened
fp_p2d_339_series_rolling_var_null_hardened
fp_p2d_345_dataframe_rolling_mean_null_hardened
fp_p2d_352_series_expanding_std_null_hardened
fp_p2d_353_series_expanding_var_null_hardened
fp_p2d_360_series_shift_null_hardened
fp_p2d_362_dataframe_astype_with_nulls_hardened
fp_p2d_363_dataframe_astype_str_to_float_hardened
fp_p2d_385_dataframe_diff_with_nulls_hardened
fp_p2d_387_dataframe_rank_with_nulls_hardened
fp_p2d_389_series_str_len_with_nulls_hardened
fp_p2d_412_series_str_rfind_with_nulls_hardened
fp_p2d_413_series_str_encode_with_nulls_hardened
fp_p2d_417_series_categorical_from_codes_missing_strict
fp_p2d_419_series_asof_utf8_before_start_null_strict
```

## Reproducing

```bash
python3 scripts/regenerate_fixtures.py --jobs 8 --report-json /tmp/report.json
# then group moved_unattributed by `classes`; the 81 are those whose class set is a
# subset of {"NULL_MARKER null->na_n", "KIND int64->float64"} and includes the first.
```
