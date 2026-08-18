"""`pivot_table`'s defaults, pinned to what was MEASURED rather than to prose.

br-frankenpandas-eay9h. The oracle hardcoded `dropna=False` and `sort=False`,
overriding pandas' `dropna=True` / `sort=True`, so it silently rewrote the
arguments under test. Knobs were added first and the defaults left alone, blocked
on this stated reason:

    "flipping the default changes the expectations of the seven fixtures that do
     not ask" ... "flipping sort reorders rows for all eight at once"

⚠️ THAT WAS NEVER MEASURED, AND HALF OF IT IS FALSE. Every pivot_table case that
compares against this oracle was enumerated — 8 banked fixtures under
fixtures/packets, the 3 `conformance_reshape` cases, the 4 `live_oracle` cases
(mean/sum/count/min) —
and asked both ways:

    dropna=True (pandas')  ->  0 of 15 change
    sort=True   (pandas')  ->  1 of 15 changes
                               (fp_p2d_127_dataframe_pivot_table_multi_values_strict)

The reason is boring and checkable: fourteen of the fifteen have no NaN group key,
so `dropna` cannot be observable on them. The one input that does have NaN keys is
`reshape_pivot_table_missing_keys_dropna_default_tn6qb9`, which already asks for
`True` explicitly. So `dropna` moved to pandas' default at ZERO cost and with no
fixture re-banked; `sort` did not move, because one fixture would have to be
re-banked and that is the corpus's call, not a free correction.

A/B against a prebuilt conformance binary (Rust side untouched, oracle reverted
then re-flipped) makes the direction unambiguous: 7 failures become 6, and the one
that flips to PASSING is
`conformance_reshape_pivot_table_missing_keys_dropna_default_tn6qb9` — the test
whose name always wanted this default. Nothing else moves. The remaining 6 are
pre-existing on that binary and identical in both arms.

These tests exist so that split cannot rot back into "the oracle's defaults are
whatever they happen to be": each half asserts the default AND that the knob is
still live in the other direction.
"""

from __future__ import annotations


def _frame_with_a_nan_group_key() -> dict:
    """Three rows whose third has a MISSING `row` key — the discriminator.

    Without a null key both defaults are inert, which is exactly why the corpus
    could carry a wrong default for so long without a single red fixture.
    """
    return {
        "index": [{"kind": "int64", "value": i} for i in range(3)],
        "column_order": ["row", "col", "val"],
        "columns": {
            "row": [
                {"kind": "utf8", "value": "r1"},
                {"kind": "utf8", "value": "r2"},
                {"kind": "null", "value": None},
            ],
            "col": [{"kind": "utf8", "value": "c1"}] * 3,
            "val": [
                {"kind": "float64", "value": 1.0},
                {"kind": "float64", "value": 2.0},
                {"kind": "float64", "value": 9.0},
            ],
        },
    }


def _pivot(oracle, **over) -> list:
    payload = {
        "operation": "dataframe_pivot_table",
        "pivot_index": "row",
        "pivot_columns": "col",
        "pivot_values": ["val"],
        "pivot_aggfunc": "sum",
        "frame": _frame_with_a_nan_group_key(),
    }
    payload.update(over)
    result = oracle.dispatch(__import__("pandas"), payload)
    return [str(entry.get("value")) for entry in result["expected_frame"]["index"]]


def test_absent_pivot_dropna_now_means_pandas_default_eay9h(oracle):
    """The flip itself. `['r1', 'r2', 'nan']` here means the override came back."""
    assert _pivot(oracle) == ["r1", "r2"], (
        "with no pivot_dropna key the oracle must use pandas' dropna=True and drop "
        "the NaN group key. An 'nan' entry means the historical dropna=False "
        "override is back and the oracle is again rewriting the argument under test"
    )


def test_pivot_dropna_false_is_still_reachable_and_still_differs_eay9h(oracle):
    """Non-vacuity, and the half that keeps the flip honest.

    Without this, `test_absent_...` above would pass just as happily if the knob
    stopped working entirely and everything returned pandas' answer — the default
    would be untested and the deliberate-override case would be silently dead.
    """
    assert _pivot(oracle, pivot_dropna=False) == ["r1", "r2", "nan"]
    assert _pivot(oracle, pivot_dropna=True) == ["r1", "r2"]
    assert _pivot(oracle, pivot_dropna=False) != _pivot(oracle)


def test_pivot_sort_default_is_deliberately_not_pandas_yet_eay9h(oracle):
    """`sort` stayed put, and that is a decision — so it is asserted, not assumed.

    If someone flips `sort` for symmetry with `dropna`, this fails and points at
    the one fixture that has to be re-banked first. It is not claiming sort=False
    is CORRECT; it is claiming the corpus has not yet chosen to pay for the change.
    """
    from pandas_oracle import _pivot_dropna, _pivot_sort

    assert _pivot_dropna({}) is True, "dropna default is pandas' (measured free)"
    assert _pivot_sort({}) is False, (
        "sort is still the historical override. Flipping it changes "
        "fp_p2d_127_dataframe_pivot_table_multi_values_strict, which must be "
        "re-banked as an explicit corpus decision — see br-frankenpandas-eay9h. "
        "Measured blast radius: 1 of 15 cases, NOT the 8 of 8 the bead assumed"
    )
    assert _pivot_sort({"pivot_sort": True}) is True, "the knob must still be live"
