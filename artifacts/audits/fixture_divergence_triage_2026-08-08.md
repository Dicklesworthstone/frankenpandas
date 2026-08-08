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

**In pandas the canonical missing value is a property of the COLUMN DTYPE, not of the input.**
Measured on live pandas 2.2.3 — every one of these was given `None` in the input:

```
object            -> element types ['str', 'NoneType', 'str']    None PRESERVED
float64           -> ['float', 'float', 'float']                 became NaN
Int64 (nullable)  -> ['int', 'NAType', 'int']                    became pd.NA
datetime64[ns]    -> ['Timestamp', 'NaTType']                    became NaT
timedelta64[ns]   -> ['Timedelta', 'NaTType']                    became NaT
```

FrankenPandas instead **preserves the input's null kind irrespective of the column's dtype**, so a
`NullKind::Null` survives inside a float64 column where pandas can only hold NaN. That is why the
round-trip and op-introduced halves are the same bug: pandas does not care which one it was, it
normalizes to the dtype either way.

The `KIND int64->float64` half of the signature is the same rule's other consequence: introducing a
missing value into an int column forces float64 in pandas, because numpy int64 cannot hold a
missing value at all.

Two independent confirmations, both measured:

```python
pd.DataFrame.from_records([{'a':1},{'a':2}], columns=['a','z'])
#    a    z          <- absent column 'z' is an all-NaN FLOAT64 column,
# 0  1  NaN             not a "null-kind" column
# dtypes: a int64, z float64

pd.Series([1,2], index=[0,1]).reindex([0,1,2])
# [1.0, 2.0, nan] float64   <- int64 PROMOTED, missing is NaN
```

Concrete corpus example, `fp_p2d_018_dataframe_from_records_column_order_new_column_null_hardened`:

```
PINNED  z: [{"kind":"null","value":"null"}, {"kind":"null","value":"null"}]
ORACLE  z: [{"kind":"null","value":"na_n"}, {"kind":"null","value":"na_n"}]
```

## This does NOT contradict lufpu / joeff

`br-frankenpandas-str-null-kind-identity-lufpu` fixed string ops to PRESERVE `None`, and
`br-frankenpandas-joeff` deliberately made null kinds distinct with kind-sensitive `Eq`/`Hash`.
Both are correct and remain correct: they concern **object-dtype** columns, which are the one case
where pandas genuinely preserves `None`. The rule is dtype-dependent; FrankenPandas applies
"preserve" universally, which is right for object and wrong for float64 / Int64 / datetime64 /
timedelta64.

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
same as deciding the remedy, and the remedy is a real FrankenPandas change (derive a column's
missing kind from its dtype, and promote int64 -> float64 when a missing value is introduced) — not
a fixture rewrite. Regenerating them first would bank pandas' answer while FP still produces the old
one, turning 81 currently-honest red rows into 81 green lies. Filed as its own bead.

## Reproducing

```bash
python3 scripts/regenerate_fixtures.py --jobs 8 --report-json /tmp/report.json
# then group moved_unattributed by `classes`; the 81 are those whose class set is a
# subset of {"NULL_MARKER null->na_n", "KIND int64->float64"} and includes the first.
```
