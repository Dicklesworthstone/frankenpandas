# p6srr — the bead's premise is wrong for part of the corpus: those fixtures were never generated

**Agent:** FuchsiaBass · **Date:** 2026-08-06 · **Bead:** br-frankenpandas-fixture-corpus-stale-vs-oracle-p6srr
**Follows:** `p6srr_class_attribution_2026-08-06.md` (the class-splitting turn)
**Machine-readable:** `artifacts/audits/p6srr_move_classes_2026-08-06.json`

Continuing the attribution pass into the 35 op-introduced null markers and the 43-fixture `VALUE`
class. Nothing attributed, regenerated, or retired; all 159 still fail; the 110 oracle errors
remain untriaged and are not folded into any total.

## HEADLINE — 94% of what looks like corpus drift is not corpus drift

**163 fixtures move against the current oracle. Only 2 of them are stale.**

The number that gets carried forward from this bead is 163, so here is what those 163 actually are.
Every moved fixture was additionally run through the oracle its own `oracle_script_sha256` NAMES,
and the verdicts partition the set exactly:

| verdict | n | share | what it means |
|---|---:|---:|---|
| `PROVENANCE_FICTION` | 96 | 58.9% | neither the named nor the current oracle produces the pinned values |
| `STAMP_IMPOSSIBLE` | 56 | 34.4% | the named oracle has no handler for the op, so it cannot have generated it |
| `BOTH_MOVED` | 8 | 4.9% | the named oracle produces a third answer |
| `GENUINELY_STALE` | **2** | **1.2%** | the named oracle DID produce these values; today's does not |
| `STAMP_ERRORED` | 1 | 0.6% | the named oracle failed for another reason — indeterminate |
| **total** | **163** | **100%** | |

- **152 of 163 — 93.3% — are PROVABLY not drift** (`PROVENANCE_FICTION` + `STAMP_IMPOSSIBLE`).
  Counting the one indeterminate `STAMP_ERRORED` gives 153, **93.9%**.
- **161 of 163 — 98.8% — do not match their named oracle at all**, so "the pinned values were once
  correct output" is false for all but two.
- **2 of 163 — 1.2% — are stale** in the sense this bead's title assumes:
  `fp_p2d_025_series_clip_with_nulls_hardened` and `fp_p2d_130_dataframe_clip_nulls_hardened`.

So the remedy the bead names — regenerate the corpus — applies to two fixtures. Applied to the
other 161 it would overwrite hand-authored parity targets with the output of the code under test.

**Why the partition is trustworthy.** Each verdict path is exercised by a test, the verdicts are
asserted to partition the moved set at runtime (not merely summed for display), and the check that
makes the newly-visible cases visible carries a negative control: disabling
`test_oracle_success_contradicts_a_fixture_that_requires_failure`'s branch turns the suite red —
**1 failed, 28 passed**, that one test and no other. A partition whose members are only ever
counted, never contradicted, is bookkeeping; this one fails loudly when the tooling and the corpus
disagree, in whichever direction they disagree.

## The finding that reframes the bead

p6srr is titled *"the corpus is stale against its oracle"*. That phrasing presumes the pinned
values were once the named oracle's output and it has since moved. **For the concat family the
presumption is false, and it is provable.**

`fp_p2d_028_dataframe_concat_axis1_basic_strict` pins int64 values with `"null"` markers. Its
`oracle_script_sha256` is `f38b2fca36d3f1…`, and that sha is **exactly** `sha256sum` of
`pandas_oracle.py` at `9aa1ed6fe` (2026-04-22) — so the stamp names a real, identifiable oracle,
and names it honestly. Extract that oracle and run it on that fixture today, on the pinned pandas
2.2.3:

    generation-era oracle (f38b2fca…) -> a=[1.0, 2.0, na_n]   b=[na_n, 10.0, 20.0]   float64
    current oracle        (9f314d86…) -> a=[1.0, 2.0, na_n]   b=[na_n, 10.0, 20.0]   float64
    the fixture pins           ->        a=[1, 2, null]       b=[null, 10, 20]       int64

**The two oracles agree with each other and neither produces the fixture.** So this is not drift.
The values were authored, not generated, and the provenance stamp certifies nothing about them.

Why the corpus holds that answer anyway: it is the **nullable `Int64`** result. Probed live,
`pd.concat(axis=1)` on plain `int64` inputs gives float64 + `NaN`; on `Int64` inputs it gives
`Int64` + `pd.NA`. DISC-011 names `FP-P2D-028` explicitly as a WILL-FIX target where FP lacks the
nullable extension dtype. **These fixtures pin the parity target FP has not implemented yet.**
Regenerating them would delete the target and turn a tracked WILL-FIX into silent agreement.

The frame payload has no per-column dtype field — `dataframe_from_json` infers it from value kinds
via `series_dtype_for_payload_values`, and int64-kinds-with-no-nulls infers plain `int64`. The
encoding simply cannot express "this column is nullable Int64", which is why no oracle can
reproduce the pinned answer.

### This is now a tool verdict, not an assertion

`regenerate_fixtures.py --provenance-oracle <path>` runs each moved fixture through the oracle its
own stamp names and splits the moves three ways:

| verdict | meaning | remedy |
|---|---|---|
| `GENUINELY_STALE` | the named oracle DID produce the pinned values; today's does not | regeneration, once the change is understood |
| `PROVENANCE_FICTION` | neither oracle produces them — authored, not generated | **regenerating destroys evidence** |
| `BOTH_MOVED` | the named oracle produces a third answer | two stacked changes; needs its own look |

Run over all 1258 fixtures with the generation-era oracle extracted from `9aa1ed6fe`:

| verdict | n | share of the 163 moved |
|---|---:|---:|
| `PROVENANCE_FICTION` | 96 | 58.9% |
| `STAMP_IMPOSSIBLE` | 56 | 34.4% |
| `BOTH_MOVED` | 8 | 4.9% |
| `GENUINELY_STALE` | **2** | **1.2%** |
| `STAMP_ERRORED` | 1 | 0.6% |

(Restated against the same pass as the bucket table below, so the two are comparable. An earlier
pass measured 93/55/8/2/1 over 159 moved — the same conclusion; the four extra moved fixtures are
the newly-visible expected-error ones plus the conformance lane's concurrent oracle fixes.)

**Two.** Out of 163 moved fixtures, two are stale in the sense the bead's title assumes:
`fp_p2d_025_series_clip_with_nulls_hardened` and `fp_p2d_130_dataframe_clip_nulls_hardened`, both
`KIND float64->int64` + `NULL_MARKER na_n->null`. Everything else pins values its own named oracle
never produced.

`STAMP_IMPOSSIBLE` is the sharper half of that: for 55 fixtures the named oracle answers
`unsupported operation` — it does not implement the operation the fixture exercises, so it cannot
have generated it, and no value comparison is needed to know that. The generation-era file is 5002
lines against today's 7985; those operations were written later. A fixture cannot have been
generated on 2026-04-22 by an oracle that gained its handler afterwards.

These non-answers are REPORTED, never dropped. An earlier version of this tool folded both refusal
cases into a silent `None` and the 56 vanished from the verdict table — the same silent
non-comparison that this tool exists downstream of.

Per-class, the fiction is not concentrated anywhere; it is the general condition:

    NULL_MARKER null->na_n   48 fiction /  2 both-moved / 35 stamp-impossible
    KIND int64->float64      39 fiction /  4 both-moved / 14 stamp-impossible
    VALUE                    20 fiction /  4 both-moved / 18 stamp-impossible / 1 stamp-errored

## `VALUE` (43) is the most heterogeneous class: 38 distinct operations

It is not a mechanism at all; it is the residue of everything that is neither a dtype nor a marker
move. Two coherent sub-families were probed.

### Datetime label spelling — a THIRD member of the `label_to_json` asymmetry family

`label_to_json` has branches for bool and int and then falls through to `str(value)`. Its own
comment already documents this family: the bool branch was added by 6bqfr, and the float branch is
deliberately deferred to its own bead. **`Timestamp` is a third member and is not mentioned.** A
datetime label enters as `utf8` in whatever spelling the fixture wrote and leaves in whatever
spelling the handler happens to use:

- `op_series_at_time` (line ~2029) pre-formats with `label_to_json(v.isoformat())` → `T` separator.
- `op_dataframe_at_time` (line ~5612) routes through `dataframe_to_json` → `str(Timestamp)` → space.

Only 2 handlers take the `isoformat` route; ~97 `dataframe_to_json` call sites take the other. Both
fixtures below are pure round-trips — `at_time` selects rows and must not rewrite labels — and the
oracle inverts the separator in **both directions**:

    fp_p2d_088_series_at_time_match_strict      input & pinned '2024-01-15 09:30:00' -> oracle '…T09:30:00'
    fp_p2d_095_dataframe_at_time_iso_format_h.  input & pinned '2024-01-15T10:30:00' -> oracle '… 10:30:00'

Same op family, opposite verdicts from one encoder gap. Affects `series_at_time`,
`series_between_time`, `dataframe_at_time`, `dataframe_between_time`, and the `+00:00` vs `Z` form
in `csv_read_frame_parse_dates_mixed_timezone`. Round-trip is impossible by construction for one of
the two spellings until `label_to_json` gains a datetime branch and the corpus picks one form.

### `memory_usage` — a dropped option, and a second provenance fiction

`fp_p2d_364_dataframe_memory_usage_with_nulls_hardened` pins `[32, 32, 234]`; both oracles emit
`[32, 32, 32]`. The handler reads `deep = payload.get("memory_usage_deep", False)` — and the
fixture payload has **no such key**, so `deep=False` and the object column reports its 8-byte
pointer array. `234` is the `deep=True` answer, and it is the exact number written in DISC-015's
own prose ("pandas returns 234 bytes"). The generation-era handler is byte-identical, so it cannot
have produced `234` either.

**So the fixture was authored from the discrepancy note rather than generated** — the same
provenance fiction as the concat family, reached by a different route (a missing payload option
instead of an unrepresentable dtype). It is the LATENT form of the dropped-option-key mechanism:
the fixture encodes an option its payload never expressed.

Note DISC-015 is ACCEPTED-with-waiver and says FP reports 32 while pandas reports 234. The oracle
now reports 32 as well — so the fixture disagrees with the oracle *and* the waiver's framing.

### The remaining `VALUE` sub-families, probed

Each ran against live pandas 2.2.3 before any verdict. The class does not resolve one way: it is
one oracle bug, one unexpressed option, and four fixture-side divergences.

| fixture | live pandas 2.2.3 | verdict |
|---|---|---|
| `..._112_dataframe_groupby_ngroup_descending` | `ngroup(ascending=False)` → `[2,1,2,0,1,2]` = **the pinned values** | **oracle wrong** |
| `..._091_series_to_datetime_unix_epoch` | bare ints → **ns**; the pinned values need `unit='s'` | option never expressed |
| `..._171_series_str_capitalize_unicode` | `'ßharp'.capitalize()` → `'Ssharp'`; fixture pins `'SSharp'` | oracle right |
| `..._419_series_asof_datetime_like_before_start_nat` | `.asof(0)` → **float `nan`**; fixture pins `na_t` | oracle right |
| `..._341_series_ewm_mean_null` | `ewm(span=3).mean()` → `[1.0, 1.0, 2.6, …]`; fixture pins `2.0` at [2] | oracle right |
| `..._127_dataframe_get_dummies_column_order` | dummies **appended last**: `['id','size','color_blue','color_red']` | oracle right |

**`groupby_ngroup` is a 6th instance of the dropped-option-key mechanism, and it is ACTIVE.**
`op_dataframe_groupby_ngroup` (line ~5560) calls `frame.groupby(columns).ngroup()` and never reads
the payload's `sort_ascending: false`, so it returns the ascending answer. The fixture is right.
Note it slips past `test_payload_keys_are_read.py` for exactly the reason that guard's docstring
gives: `sort_ascending` appears 13 times elsewhere in the oracle, and the guard only asks whether a
key appears *anywhere*.

**`to_datetime` on bare integers is the LATENT form, and it points at a real FP divergence.**
The payload carries no unit key at all, yet the pinned values are the `unit='s'` interpretation;
pandas reads bare ints as **nanoseconds**. So the fixture pins an option it never sent — the same
shape as the `memory_usage` `deep` case — and separately implies FrankenPandas interprets integer
`to_datetime` input as seconds. That second half is a parity question worth its own bead, not a
corpus question.

The four "oracle right" rows are FrankenPandas behaviours pinned as expectations:

- **`str.capitalize` on `ß`** — Python/pandas titlecase the first character (`'Ss'`); Rust's
  `char::to_uppercase('ß')` yields `"SS"`, so a `first.to_uppercase() + rest.to_lowercase()`
  implementation produces `'SSharp'`. A real casing divergence, and `'straße'`/`'élan'`/`'maçã'` in
  the same fixture all agree — only the leading `ß` splits.
- **`Series.asof` before the start** — pandas returns float `nan` even for a datetime64 series
  (probed both object and datetime64 dtype); FP returns `NaT`, preserving dtype. FP's answer is
  arguably better and is still a divergence.
- **`ewm(span=3).mean()` with nulls** — pandas gives `2.6` where the fixture pins `2.0`, so FP's
  null handling in the EWM recurrence differs.
- **`get_dummies` column placement** — pandas appends dummy columns after all remaining originals;
  FP substitutes them in place.

None of these four is attributable to a *named* divergence today: DISCREPANCIES.md has no entry for
casing, asof missing-value dtype, EWM null handling, or dummy column placement. They are candidate
new entries, and each needs the FP side confirmed before anything is written down.

## The 110 oracle errors, triaged — and a false-agreement in this tool

The bead's standing precondition was that nothing is retired until these are triaged, "several
being legitimate expected-error fixtures". They are, and it is most of them.

94 fixtures pin `expected_error_contains`. **85 of the reported errors are those fixtures failing
exactly as they assert** — both sides agree the operation fails, so they are agreements, not
defects. That leaves **27 genuinely untriaged**, not 110.

**The error TEXT is deliberately not compared.** `expected_error_contains` pins FrankenPandas's
wording, which the Rust harness checks; the oracle raises pandas' own English. My first pass at
this triage matched them as substrings and rejected 45 of the agreements — on cases like a fixture
expecting `'out of bounds'` against pandas' `'positional indexers are out-of-bounds'`. A hyphen.
The predicate is the *existence* of an error expectation, never its text, and a test pins that.

The remaining 9 are the find: **expected-error fixtures where the oracle SUCCEEDS.**

    fp_p2c_010_series_filter_non_boolean_mask_strict
    fp_p2d_016_csv_round_trip_ragged_row_error_strict
    fp_p2d_022_dataframe_constructor_list_like_empty_rows_nonempty_index_error_strict
    fp_p2d_023_dataframe_constructor_list_like_dtype_bool_invalid_int_error_strict
    fp_p2d_024_dataframe_from_dict_dtype_category_unsupported_error_hardened
    fp_p2d_025_dataframe_iloc_duplicate_column_selector_error_strict
    fp_p2d_046_series_fillna_cast_error_strict
    fp_p2d_245_series_dt_total_seconds_basic_strict
    fp_p2d_423_series_str_split_expand_empty_separator_strict

Every one was being counted as `agree, provenance-only`, because `expected_error_contains` was
filed as an UNCOMPARED key. **A fixture asserting "this must fail" was silently satisfied by a run
that did not fail** — the silent-non-comparison-counted-as-success bug, inside the tool built to
catch it. They are now moves, classed `ERROR expected-but-succeeded`. As a class they say FP raises
where pandas returns a value, which is a real over-strictness question and was invisible.

### Corrected buckets (live pandas 2.2.3, all 1258)

    agree, provenance-only     : 966
    MOVED, unattributed        : 163   <-- all still failing
    oracle: unsupported op     :  17
    expected-error, BOTH failed:  85   <-- legitimate, NOT defects
    oracle: other errors       :  27   <-- UNTRIAGED, deliberately not folded into any total
    ------------------------------------
                                 1258   every fixture in exactly one bucket

`uncompared_keys` is now **empty**: no key reaches the end of a run unadjudicated. A bare-null
`expected_*` key is also no longer counted — generated fixtures carry every key with a null
placeholder, and one was inflating the uncompared count to 12 when only 11 fixtures made the
assertion. Missing values are encoded `{"kind":"null",…}`, never bare JSON null, so nothing real is
swallowed.

⚠️ **Cross-pass deltas in this document are not all attributable to my changes.** The conformance
lane is landing oracle fixes in the same session. Two fixtures I counted ad hoc as
expected-but-succeeded — `..._025_dataframe_iloc_missing_column_error_hardened` and its `loc`
sibling — are now correct agreements because of `a1239a9f0` (loc/iloc apply the `column_order`
selector). My ad-hoc count of 11 was taken before that commit; the tool's 9 is after it. Compare
buckets within one pass, not across passes.

## The 27 remaining errors, triaged into families

The last untriaged bucket. Nine families, and none is a mystery:

| n | family | reading |
|---:|---|---|
| 10 | reductions raise on object columns | **belongs to `zx21n`** |
| 4 | `constructor_list_like` / `from_records` column-count mismatch | fixture expects null-padding of ragged rows; pandas raises |
| 2 | oracle dtype policy causes a hard error | see below |
| 2 | `str.startswith`/`endswith` empty pattern | **oracle over-restriction** |
| 2 | `dt.to_pydatetime(warn=…)` | kwarg does not exist in pinned pandas 2.2.3 |
| 2 | `dataframe_asof` `'<' not supported between int and Timestamp` | oracle builds mismatched index/label types |
| 2 | `xs` single-match | oracle self-declared limitation ("currently requires…") |
| 2 | `series_filter` null mask | pandas refuses a mask with NA; fixture expects a value |
| 1 | `from_series` duplicate name non-identical values | oracle refuses to represent it |

**The 10 reductions are `zx21n`'s mechanism one op-family over, with the opposite symptom.**
`zx21n` records that `sum`/`min`/`max` behave as `numeric_only=True` while pandas 2.x includes
object columns — a *concealed* agreement. For `mean`, `median`, `std`, `var`, `sem`, `skew` (×2),
`kurtosis`, `quantile` and `prod`, pandas 2.x does not silently include the object column, it
**raises** (`could not convert string to float: 'alpha'`). Same default, two symptom classes: three
fixtures conceal it, ten more surface it as an oracle error. Reported on `zx21n`; not closing a
peer's bead.

**The oracle's dtype policy does not only move values — it makes the oracle fail outright.**
This is a third symptom of the `series_dtype_for_payload_values` mechanism, after moved values (45
fixtures) and unrepresentable null markers (50):

- `fp_p2d_056_dataframe_merge_asof_backward_nan_left_key`: the left `time` column has a null so it
  infers nullable `Int64`; the right `time` has none so it infers plain `int64`; `merge_asof` then
  raises `incompatible merge keys … must be the same type`. Reproduced exactly.
- `fp_p2d_025_series_clip_array_bounds_missing_entries`: the bounds series carries a null → `Int64`,
  and clip's `-inf` fill cannot be stored in it → `Invalid value '-inf' for dtype Int64`.

⚠️ **But fixing the dtype policy would not rescue the `merge_asof` fixture.** Probed: with both
sides forced to a uniform `float64`, pandas raises a *different* error — `Merge keys contain null
values on left side`. **pandas refuses a null merge key under any dtype**, so the fixture pins a
frame pandas will never produce. The dtype mismatch is masking the real divergence rather than
being it. Worth stating because the obvious fix here looks like it closes a fixture and does not.

**`str.startswith`/`endswith` with an empty pattern is an oracle over-restriction.** The handler
rejects an empty `regex_pattern` outright; pandas is happy to answer (`s.str.startswith('')` is
`True` everywhere). The fixture is legitimate and the oracle refuses to run it.

## What is NOT concluded

- **Nothing attributed.** All 163 stay failing.
- **The 27 are now triaged into families above, but none is RESOLVED.** They stay in the
  `other_errors` bucket and out of every agreement total until each family's owner acts: 10 belong
  to `zx21n`, the oracle-side ones (`startswith`/`endswith`, `to_pydatetime(warn=)`, `asof` type
  mismatch, `xs`) belong to the conformance lane's file, and the rest need the FP side run.
  Triaged is not fixed, and a family label is not an attribution.
- The 9 `ERROR expected-but-succeeded` fixtures are newly VISIBLE, not newly resolved. Each needs
  the FP side run before anyone decides whether FP is over-strict or pandas is too permissive.
- The four fixture-side divergences above are probed against pandas but **not yet confirmed against
  FrankenPandas itself** — the pinned values are consistent with FP behaviour, which is not the same
  as having run FP. That confirmation is the next step before any DISCREPANCIES entry.
- Whether the corpus should adopt nullable `Int64`/`Float64` payload dtypes — the encoding gap
  underneath both the concat family and the 50 round-trip markers — is a design decision for the
  maintainer, not something to settle by regenerating.
