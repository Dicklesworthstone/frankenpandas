# p6srr — the two largest move classes are NOT one mechanism each

**Agent:** FuchsiaBass · **Date:** 2026-08-06 · **Bead:** br-frankenpandas-fixture-corpus-stale-vs-oracle-p6srr
**Machine-readable source:** `artifacts/audits/p6srr_move_classes_2026-08-06.json`
**Prior turn:** `abbaa07e6` refuted DISC-011 for the int64→float64 class using a groupby fixture.

The maintainer's standing rule is that a named divergence is a HYPOTHESIS until a representative
fixture reproduces it against live pandas, and that a class must not take one attribution if it
holds more than one mechanism. This turn tested exactly that on the two biggest classes. **Both
split, and in the int64→float64 case the halves have OPPOSITE verdicts.** Nothing has been
regenerated, retired, or attributed.

## Corpus state (dry run, live system pandas 2.2.3, all 1258 fixtures)

    agree, provenance-only : 972      MOVED, unattributed : 159
    oracle: unsupported op : 17       oracle: other errors: 110

## The reach test that motivated this turn

`abbaa07e6` traced the int64→float64 class to `op_groupby_agg`, which forces
`value_dtype='float64'` for every agg outside {min,max,first,last}. That was a real mechanism —
but **`op_groupby_agg` only covers the groupby family, and only 2 of the 57 fixtures in the class
are groupby fixtures.** The mechanism explains 3.5% of its own class. The other 55 needed probing.

### Class `KIND int64->float64` (57) splits three ways

| Sub-class | n | Mechanism | Verdict |
|---|---|---|---|
| groupby | 2 | `op_groupby_agg` forces float64 value dtype | **oracle wrong** |
| general ops | 45 | hand-rolled series construction (below) | **oracle wrong** |
| `series_dt_*` | 10 | pandas genuinely upcasts on NaT (below) | **corpus is FP-adapted** |

**The 45: 51 oracle call sites bypass the corpus's own dtype contract.**
`series_dtype_for_payload_values` maps int64-kinds+nulls to nullable `Int64`. 69 call sites apply
it via `fixture_series_from_payload`; **51 hand-roll `pd.Series(values, index=index, name=...)`
with no `dtype=`**, so those payloads infer `float64`. Probed on
`fp_p2d_046_series_fillna_numeric_basic_strict`:

    pd.Series([1,None,nan,4,None], dtype='Int64').fillna(0) -> Int64   [1,0,0,4,0]  == the pinned fixture
    pd.Series([1,None,nan,4,None]).fillna(0)                -> float64 [1.0,0.0,..] == what the oracle emits

Same shape as the groupby finding: the oracle overrides its input rather than modelling pandas.
The pinned fixtures are correct.

**The 10 `series_dt_*`: this half inverts, and one fixture carries a defect AND a divergence.**
`.dt.microsecond` and siblings return int32 only when no NaT is present; **with any NaT pandas
returns float64**. These fixtures pin int64 + a null marker, which is FP's masked-int model, not
pandas'. Here the ORACLE is right and the corpus is adapted-to-FP — a genuine divergence for
DISCREPANCIES.md, not a regeneration.

Compounding it, a *separate* oracle defect sits on the same fixtures: `8d78992a6` fixed
`format="mixed"` on `series_to_datetime` and **not on `op_series_dt_accessor` or the other 14
`to_datetime(` call sites**, so heterogeneous fixture strings coerce to NaT. Probed on
`fp_p2d_415_series_dt_microsecond_basic_strict`:

    to_datetime(s, errors='coerce')                 -> [ts, NaT, NaT, NaT] -> [123456.0, nan, nan, nan]
    to_datetime(s, errors='coerce', format='mixed') -> [ts, ts,  ts,  NaT] -> [123456.0, 987654.0, 0.0, nan]

The `format="mixed"` VALUES match the pinned fixture; the DTYPE does not. One fixture, two
findings, opposite directions. Neither can be resolved by regenerating.

### Class `NULL_MARKER null->na_n` (85) splits 50 / 35

The suspicion that this was an oracle change rather than fixture staleness is **confirmed for 50
of the 85, provable without pandas at all.**

`fp_p2c_010_series_head_with_nulls_hardened` pins `{"kind":"null","value":"null"}` at values[1] of
**both its input and its expectation**. `head(3)` does not touch that element. The oracle emits
`na_n`. It cannot return a marker it was handed.

Root cause is an asymmetry inside `series_dtype_for_payload_values` itself: int64+null →
nullable `Int64` (missing round-trips as `pd.NA` → `"null"`), but float64+null → plain `float64`
(missing collapses to `nan` → `"na_n"`). Probed: `dtype="Float64"` preserves the marker. This is
the bool-label read/write asymmetry fixed in 6bqfr, one dtype family over.

The remaining 35 had **no null marker in their input** — the missing value was introduced by the
operation (outer merge, reindex, alignment). Nothing here says who is right for those; they still
need their own live-pandas probe and must not inherit the 50's attribution. Same split applies to
`NULL_MARKER na_n->null`: 5 round-trip, 4 op-introduced.

**This discriminator is now computed by the tool**, not asserted here:
`regenerate_fixtures.roundtrip_implicated` reports it per fixture and per class, so an attribution
pass cannot merge the two halves by accident.

## A correction to this tool's own class counts

`SHAPE key-added-by-oracle` was **54, and is 7**. Most fixtures store no `column_order` while the
live oracle always emits one, and the classifier was counting that leaf — but
`fixture_differ.frame_column_order` deliberately treats an absent or empty `column_order` as "no
ordering claim" and compares the column SET instead. 47 fixtures were labelled with a mechanism
the adjudicator does not consider a difference, and the example line showed that non-difference as
the exemplar of the move. Counting a default against a populated value is the same bug that
produced 269 of the differ's 420 phantom rows; the classifier now mirrors the adjudicator's rule
instead of re-deriving one. Total labels: 289 → 242.

## What is NOT concluded

- **Nothing is attributed.** All 159 stay failing. Two mechanisms are now oracle defects with live
  probes behind them, but the fixes belong to `pandas_oracle.py`, which is the conformance lane's
  file — reported to LavenderSnow, not edited here.
- **The 110 oracle errors are still untriaged**, several being legitimate expected-error fixtures.
  Per the standing rule, nothing is retired or regenerated until that triage lands.
- **The 35 op-introduced null markers and the 43-fixture `VALUE` class are unprobed.**
- `KIND float64->int64` fell 10 → 6 during this session as the conformance lane landed
  `resolve_constructor_dtype` fixes; the residual 6 are not re-analysed here.
