# p6srr — the bead's premise is wrong for part of the corpus: those fixtures were never generated

**Agent:** FuchsiaBass · **Date:** 2026-08-06 · **Bead:** br-frankenpandas-fixture-corpus-stale-vs-oracle-p6srr
**Follows:** `p6srr_class_attribution_2026-08-06.md` (the class-splitting turn)
**Machine-readable:** `artifacts/audits/p6srr_move_classes_2026-08-06.json`

Continuing the attribution pass into the 35 op-introduced null markers and the 43-fixture `VALUE`
class. Nothing attributed, regenerated, or retired; all 159 still fail; the 110 oracle errors
remain untriaged and are not folded into any total.

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

| verdict | n | share of the 159 moved |
|---|---:|---:|
| `PROVENANCE_FICTION` | 93 | 58.5% |
| `STAMP_IMPOSSIBLE` | 55 | 34.6% |
| `BOTH_MOVED` | 8 | 5.0% |
| `GENUINELY_STALE` | **2** | **1.3%** |
| `STAMP_ERRORED` | 1 | 0.6% |

**Two.** Out of 159 moved fixtures, two are stale in the sense the bead's title assumes:
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

    NULL_MARKER null->na_n   48 fiction /  2 both-moved / 35 no-answer
    KIND int64->float64      39 fiction /  4 both-moved / 14 no-answer
    VALUE                    20 fiction /  4 both-moved / 19 no-answer

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

## What is NOT concluded

- **Nothing attributed.** All 159 stay failing.
- **The 110 oracle errors are untriaged** and deliberately excluded from every total above.
- The remaining `VALUE` sub-families are unprobed: `to_datetime` unit/origin precision
  (`1704067200000000000` vs `1704067200`), `get_dummies` column ordering, `str.capitalize` on `ß`
  (`'SSharp'` vs `'Ssharp'`), `ewm_mean` with nulls, `groupby_ngroup` descending, and the
  `na_t`→`na_n` marker in `series_asof`.
- Whether the corpus should adopt nullable `Int64`/`Float64` payload dtypes — the encoding gap
  underneath both the concat family and the 50 round-trip markers — is a design decision for the
  maintainer, not something to settle by regenerating.
