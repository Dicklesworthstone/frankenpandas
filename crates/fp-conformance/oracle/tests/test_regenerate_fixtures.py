"""Tests for the round-trip discriminator in scripts/regenerate_fixtures.py.

Per br-frankenpandas-fixture-corpus-stale-vs-oracle-p6srr. The tool refuses to
regenerate a fixture whose move is not attributed to a named divergence, so the
attribution pass runs by CLASS -- and the largest class,
`NULL_MARKER null->na_n` at 85 fixtures, turned out to hold two mechanisms with
OPPOSITE verdicts. `roundtrip_implicated` is what separates them, so it is the
thing under test here.

The negative cases are the point. A naive implementation that answers "yes" for
every null-marker move, or that scans the pinned expectation as well as the
input, or that ignores direction, passes the positive case and would have
attributed all 85 to one story.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[4]
SCRIPTS = PROJECT_ROOT / "scripts"
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

FIXTURE_ROOT = PROJECT_ROOT / "crates/fp-conformance/fixtures/packets"

regenerate_fixtures = pytest.importorskip(
    "regenerate_fixtures",
    reason="regenerate_fixtures imports fixture_differ from the oracle dir",
)

NULL = {"kind": "null", "value": "null"}
NAN = {"kind": "null", "value": "na_n"}
NULL_TO_NAN = "NULL_MARKER null->na_n"


def series(*values: dict) -> dict:
    return {
        "name": "s",
        "index": [{"kind": "int64", "value": i} for i in range(len(values))],
        "values": list(values),
    }


def test_input_markers_come_from_inputs_only_not_the_pinned_answer():
    """NEGATIVE: reading `expected*` would make every move look like round-trip."""
    fixture = {
        "operation": "series_head",
        "left": series({"kind": "float64", "value": 1.0}),
        "expected_series": series(NULL),
    }
    assert regenerate_fixtures.input_null_markers(fixture) == set()


def test_input_markers_ignore_provenance_and_retirement_blocks():
    fixture = {
        "left": series({"kind": "float64", "value": 1.0}),
        "fixture_provenance": {"note": {"kind": "null", "value": "null"}},
        "retired": {"reason": {"kind": "null", "value": "null"}},
    }
    assert regenerate_fixtures.input_null_markers(fixture) == set()


def test_input_markers_are_found_in_any_input_payload():
    fixture = {
        "left": series({"kind": "int64", "value": 1}),
        "right": series(NAN),
        "fill_value": NULL,
    }
    assert regenerate_fixtures.input_null_markers(fixture) == {"na_n", "null"}


def test_nan_and_na_n_are_the_same_marker():
    """`scalar_from_json` decodes both to float('nan'); they must not read apart."""
    fixture = {"left": series({"kind": "null", "value": "nan"})}
    assert regenerate_fixtures.input_null_markers(fixture) == {"na_n"}
    assert regenerate_fixtures.roundtrip_implicated(
        fixture, ["NULL_MARKER nan->null"]
    ) == ["NULL_MARKER nan->null"]


def test_implicated_when_the_pinned_marker_was_handed_to_the_oracle():
    fixture = {"left": series({"kind": "float64", "value": 1.0}, NULL)}
    assert regenerate_fixtures.roundtrip_implicated(fixture, [NULL_TO_NAN]) == [
        NULL_TO_NAN
    ]


def test_not_implicated_when_the_operation_introduced_the_missing_value():
    """NEGATIVE: outer merges and reindexes CREATE nulls. Different mechanism."""
    fixture = {
        "left": series({"kind": "int64", "value": 1}),
        "right": series({"kind": "int64", "value": 2}),
    }
    assert regenerate_fixtures.roundtrip_implicated(fixture, [NULL_TO_NAN]) == []


def test_direction_is_load_bearing():
    """NEGATIVE: an input carrying only `na_n` does not explain a pinned `null`."""
    fixture = {"left": series(NAN)}
    assert regenerate_fixtures.roundtrip_implicated(fixture, [NULL_TO_NAN]) == []
    assert regenerate_fixtures.roundtrip_implicated(
        fixture, ["NULL_MARKER na_n->null"]
    ) == ["NULL_MARKER na_n->null"]


def test_non_null_marker_classes_are_never_implicated():
    """NEGATIVE: a dtype move is not a marker round-trip, whatever the inputs hold."""
    fixture = {"left": series({"kind": "int64", "value": 1}, NULL)}
    assert regenerate_fixtures.roundtrip_implicated(
        fixture, ["KIND int64->float64", "SHAPE key-added-by-oracle", "UNCLASSIFIED"]
    ) == []


def test_only_the_null_marker_classes_survive_a_mixed_class_list():
    fixture = {"left": series({"kind": "int64", "value": 1}, NULL)}
    assert regenerate_fixtures.roundtrip_implicated(
        fixture, ["KIND int64->float64", NULL_TO_NAN]
    ) == [NULL_TO_NAN]


def test_empty_class_list_is_not_implicated():
    fixture = {"left": series(NULL)}
    assert regenerate_fixtures.roundtrip_implicated(fixture, []) == []


# --- Anchors in the real corpus -------------------------------------------
#
# These pin the two sides of the split against actual fixtures, so the
# discriminator cannot drift away from the population it was built to describe.


def load_packet(name: str) -> dict:
    path = FIXTURE_ROOT / name
    if not path.exists():  # pragma: no cover - corpus is in-tree
        pytest.skip(f"fixture corpus missing {name}")
    return json.loads(path.read_text(encoding="utf-8"))


def test_corpus_anchor_head_with_nulls_is_a_round_trip_loss():
    """`head(3)` does not touch values[1]; input and expectation both pin `null`."""
    fixture = load_packet("fp_p2c_010_series_head_with_nulls_hardened.json")
    assert "null" in regenerate_fixtures.input_null_markers(fixture)
    assert fixture["expected_series"]["values"][1] == NULL
    assert regenerate_fixtures.roundtrip_implicated(fixture, [NULL_TO_NAN]) == [
        NULL_TO_NAN
    ]


def test_corpus_anchor_merge_missing_is_op_introduced():
    """An outer-ish merge creates the missing cell; no null marker in its input."""
    fixture = load_packet("fp_p2d_014_dataframe_merge_column_left_missing_hardened.json")
    assert regenerate_fixtures.input_null_markers(fixture) == set()
    assert regenerate_fixtures.roundtrip_implicated(fixture, [NULL_TO_NAN]) == []


# --- The classifier must not count leaves the adjudicator ignores ----------


def frame(columns: dict, **extra) -> dict:
    return {"index": [{"kind": "int64", "value": 0}], "columns": columns, **extra}


def test_absent_column_order_is_not_a_move():
    """The 61-fixture `key-added-by-oracle` class was mostly this non-difference.

    `fixture_differ.frame_column_order` treats an absent `column_order` as "no
    ordering claim" and compares the column SET instead, so a fixture that omits
    the key while the live oracle emits one has NOT moved on it.
    """
    cell = [{"kind": "int64", "value": 1}]
    pinned = frame({"a": cell})
    live = frame({"a": cell}, column_order=["a"])
    assert regenerate_fixtures.classify_move(pinned, live)[0] == ["NORMALIZES_EQUAL"]


def test_empty_column_order_is_not_a_move_either():
    """An empty list is 'not explicit' by the same rule, not a zero-length order."""
    cell = [{"kind": "int64", "value": 1}]
    pinned = frame({"a": cell}, column_order=[])
    live = frame({"a": cell}, column_order=["a"])
    assert regenerate_fixtures.classify_move(pinned, live)[0] == ["NORMALIZES_EQUAL"]


def test_a_real_reordering_between_two_explicit_orders_still_counts():
    """NEGATIVE CONTROL: the exclusion must not swallow genuine order moves.

    Both sides record an explicit, non-empty order, which is exactly the case
    `frame_equal` DOES adjudicate.
    """
    cell = [{"kind": "int64", "value": 1}]
    pinned = frame({"a": cell, "b": cell}, column_order=["a", "b"])
    live = frame({"a": cell, "b": cell}, column_order=["b", "a"])
    classes, _ = regenerate_fixtures.classify_move(pinned, live)
    assert classes != ["NORMALIZES_EQUAL"]


def test_a_real_value_move_is_untouched_by_the_column_order_exclusion():
    """NEGATIVE CONTROL: dropping the ignored leaf must not drop the real one."""
    pinned = frame({"a": [{"kind": "int64", "value": 1}]})
    live = frame({"a": [{"kind": "float64", "value": 1.0}]}, column_order=["a"])
    classes, example = regenerate_fixtures.classify_move(pinned, live)
    assert classes == ["KIND int64->float64"]
    # The exemplar must be the real move, not the ignored key.
    assert "column_order" not in example
