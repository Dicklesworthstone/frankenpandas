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


# --- expected-error fixtures ----------------------------------------------
#
# 11 fixtures assert "this operation must fail" while the oracle returns a value,
# and every one was counted as agreement because `expected_error_contains` was
# reported as an UNCOMPARED key. That is the silent-non-comparison-as-success bug
# this tool exists downstream of, occurring inside this tool.


def test_a_fixture_pinning_an_error_message_expects_an_error():
    assert regenerate_fixtures.fixture_expects_error({"expected_error_contains": "boom"})


def test_a_bare_null_placeholder_is_not_an_assertion():
    """NEGATIVE: generated fixtures carry every expected_* key with a null."""
    assert not regenerate_fixtures.fixture_expects_error(
        {"expected_error_contains": None}
    )
    assert not regenerate_fixtures.fixture_expects_error({})


def test_oracle_success_contradicts_a_fixture_that_requires_failure():
    fixture = {"expected_error_contains": "boolean mask required for filter"}
    verdict = regenerate_fixtures.classify(fixture, {"expected_series": None})
    assert verdict["moved"] == ["expected_error_contains"]
    assert "expected_error_contains" not in verdict["uncompared"], (
        "an expected-error fixture the oracle contradicted must never be filed "
        "as merely uncompared"
    )
    assert "succeeded" in verdict["detail"]["expected_error"]


def test_a_null_placeholder_is_neither_moved_nor_uncompared():
    """NEGATIVE: the placeholder must not become a phantom disagreement either."""
    verdict = regenerate_fixtures.classify(
        {"expected_error_contains": None}, {"expected_series": None}
    )
    assert verdict["moved"] == []
    assert verdict["uncompared"] == []


def test_error_text_is_not_compared_between_fp_and_pandas():
    """`expected_error_contains` pins FP's wording; the oracle raises pandas'.

    The real corpus case: a fixture expecting 'out of bounds' against pandas'
    'positional indexers are out-of-bounds'. Matching those texts would reject
    83 legitimate agreements over a hyphen.
    """
    fixture = {"expected_error_contains": "out of bounds"}
    assert regenerate_fixtures.fixture_expects_error(fixture)
    # The predicate is about EXISTENCE of an error expectation, never its text.
    assert regenerate_fixtures.fixture_expects_error(
        {"expected_error_contains": "totally different wording"}
    )


# --- "stale" vs "never generated" are different findings -------------------
#
# p6srr is titled "the corpus is stale against its oracle", which presumes the
# pinned values were once that oracle's output. For the concat family they were
# not: the fixture's own named oracle (sha f38b2fca…, = pandas_oracle.py at
# 9aa1ed6fe) emits float64 + na_n today, same as the current one, while the
# fixture pins int64 + null. Regeneration is the remedy for one and destroys
# evidence in the other, so the verdicts must never share a total.


def test_named_oracle_reproduces_the_fixture_means_genuinely_stale():
    assert (
        regenerate_fixtures.provenance_verdict(True, False)
        == regenerate_fixtures.GENUINELY_STALE
    )


def test_named_oracle_agreeing_with_today_means_the_stamp_is_fiction():
    """NEGATIVE: this is the case a 'stale corpus' framing gets wrong."""
    assert (
        regenerate_fixtures.provenance_verdict(False, True)
        == regenerate_fixtures.PROVENANCE_FICTION
    )


def test_named_oracle_agreeing_with_neither_is_its_own_bucket():
    assert (
        regenerate_fixtures.provenance_verdict(False, False)
        == regenerate_fixtures.BOTH_MOVED
    )


def test_reproducing_the_fixture_wins_over_matching_today():
    """Degenerate input: if it matches the fixture, the fixture is what matters."""
    assert (
        regenerate_fixtures.provenance_verdict(True, True)
        == regenerate_fixtures.GENUINELY_STALE
    )


def test_every_moved_fixture_gets_exactly_one_verdict():
    """The partition property the headline depends on.

    "163 moved fixtures" is the number that gets carried forward, so every
    verdict is reported as a share of it. If the verdicts did not partition the
    moved set, the shares would not add up and the headline would be arithmetic
    rather than a decomposition.
    """
    verdicts = [
        regenerate_fixtures.provenance_verdict(True, False),
        regenerate_fixtures.provenance_verdict(False, True),
        regenerate_fixtures.provenance_verdict(False, False),
        regenerate_fixtures.named_oracle_verdict("unsupported operation: 'x'"),
        regenerate_fixtures.named_oracle_verdict("something else blew up"),
    ]
    assert all(verdicts), "every path must yield a verdict, never a silent blank"
    assert len(set(verdicts)) == len(verdicts), "the paths must not collide"


def test_the_verdicts_are_distinct_labels():
    labels = {
        regenerate_fixtures.GENUINELY_STALE,
        regenerate_fixtures.PROVENANCE_FICTION,
        regenerate_fixtures.BOTH_MOVED,
        regenerate_fixtures.STAMP_IMPOSSIBLE,
        regenerate_fixtures.STAMP_ERRORED,
    }
    assert len(labels) == 5


def test_named_oracle_refusing_the_operation_is_an_impossible_stamp():
    """An oracle that cannot run the op cannot have generated the fixture."""
    assert (
        regenerate_fixtures.named_oracle_verdict(
            "unsupported operation: 'dataframe_mask_df'"
        )
        == regenerate_fixtures.STAMP_IMPOSSIBLE
    )


def test_other_named_oracle_failures_are_reported_not_dropped():
    """NEGATIVE: a crash is NOT evidence the stamp is impossible — different bucket.

    Both must still be reported. A silently dropped non-answer is the differ bug
    this tool exists downstream of.
    """
    verdict = regenerate_fixtures.named_oracle_verdict(
        "dataframe_mask_df failed: bad operand type for unary ~: 'NoneType'"
    )
    assert verdict == regenerate_fixtures.STAMP_ERRORED
    assert verdict != regenerate_fixtures.STAMP_IMPOSSIBLE
    assert verdict, "a non-answer must never be the empty string"


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
