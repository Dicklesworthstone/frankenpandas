"""Tests for fixture_differ.py's comparison core.

Per br-frankenpandas-live-oracle-passes-by-skip-l7r1p. `fixture_differ.py` is
the screening tool that answers "does the pinned corpus still agree with live
pandas". Run against the corpus it reported **420 fixture defects**; 238 of
those were the tool's own false positives, and one of the two bugs also meant a
large slice of frames had their VALUES never compared at all. A screening tool
whose number cannot be trusted is worse than no tool, because the number gets
quoted. These tests pin the two fixes.

The module is a script rather than a package, so it is loaded by path.
"""
from __future__ import annotations

import importlib.util
import pathlib
import sys

import pytest


ORACLE_ROOT = pathlib.Path(__file__).resolve().parents[1]


def _load_differ():
    spec = importlib.util.spec_from_file_location(
        "fixture_differ", ORACLE_ROOT / "fixture_differ.py"
    )
    module = importlib.util.module_from_spec(spec)
    # Register before exec: the module defines @dataclass types, and the
    # decorator resolves each class's __module__ through sys.modules. Without
    # this the import dies in dataclasses.py with a bare
    # "'NoneType' object has no attribute '__dict__'".
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


@pytest.fixture(scope="module")
def differ():
    return _load_differ()


# ---------------------------------------------------------------------------
# serde kind aliases: "str"/"string" ARE "utf8"
# ---------------------------------------------------------------------------


def test_str_and_utf8_kinds_compare_equal(differ):
    """fp-types Scalar declares #[serde(alias = "string", alias = "str")].

    Comparing the kind literally made ~25 string-op fixtures read as divergent
    from a live result carrying the identical value.
    """
    assert differ.dict_equal({"kind": "str", "value": "h"}, {"kind": "utf8", "value": "h"})
    assert differ.dict_equal({"kind": "string", "value": "h"}, {"kind": "utf8", "value": "h"})
    assert differ.dict_equal({"kind": "utf8", "value": "h"}, {"kind": "str", "value": "h"})


def test_alias_normalization_does_not_mask_a_real_value_difference(differ):
    """The negative control: same kinds, different values must still differ."""
    assert not differ.dict_equal(
        {"kind": "str", "value": "h"}, {"kind": "utf8", "value": "different"}
    )
    # ...and genuinely different kinds must still differ.
    assert not differ.dict_equal(
        {"kind": "int64", "value": 3}, {"kind": "utf8", "value": "3"}
    )
    assert not differ.dict_equal(
        {"kind": "int64", "value": 3}, {"kind": "float64", "value": 3.0}
    )


# ---------------------------------------------------------------------------
# column order: absent is not the same as empty
# ---------------------------------------------------------------------------


def test_column_order_derived_from_columns_mapping_when_absent(differ):
    """Most fixtures record only `columns`; `column_order` is simply absent.

    `.get("column_order", [])` turned that absence into an empty-list CLAIM.
    """
    frame = {"index": [], "columns": {"a": [], "b": []}}
    names, explicit = differ.frame_column_order(frame)
    assert names == ["a", "b"]
    assert explicit is False


def test_explicit_column_order_is_reported_as_explicit(differ):
    frame = {"index": [], "column_order": ["b", "a"], "columns": {"a": [], "b": []}}
    names, explicit = differ.frame_column_order(frame)
    assert names == ["b", "a"]
    assert explicit is True


def test_absent_pinned_order_no_longer_reports_a_mismatch(differ):
    """The 269-row false positive, reduced to its minimal shape."""
    pinned = {"index": [], "columns": {"a": []}}
    live = {"index": [], "column_order": ["a"], "columns": {"a": []}}
    equal, message = differ.frame_equal(pinned, live)
    assert equal, message


def test_values_are_actually_compared_when_order_was_absent(differ):
    """The silent no-op, which is the more dangerous half of that bug.

    The value loop iterates the pinned column names. When those defaulted to
    `[]`, it compared NOTHING — a frame could differ in every value and still be
    declared equal once the order check was satisfied. This asserts a real value
    difference is now caught with no `column_order` recorded on either side.
    """
    pinned = {"index": [], "columns": {"a": [{"kind": "int64", "value": 1}]}}
    live = {"index": [], "columns": {"a": [{"kind": "int64", "value": 999}]}}
    equal, message = differ.frame_equal(pinned, live)
    assert not equal
    assert "999" in message or "mismatch" in message


def test_column_set_difference_is_still_caught_without_explicit_order(differ):
    pinned = {"index": [], "columns": {"a": []}}
    live = {"index": [], "columns": {"a": [], "b": []}}
    equal, message = differ.frame_equal(pinned, live)
    assert not equal
    assert "column set mismatch" in message


def test_diff_result_carries_fixture_identity_not_just_packet_id(differ):
    """A packet id is not a fixture id, and triage is per-fixture.

    br-frankenpandas-fixture-divergence-triage-9s0c4: FP-P2D-037 alone covers
    four fixtures, of which exactly one diverges. With only the packet id
    recorded, a reported divergence could not be located, reproduced, or
    triaged — and the maintainer's regeneration policy is explicitly
    per-fixture, so the tool could not support the policy it feeds.
    """
    fields = differ.DiffResult.__dataclass_fields__
    assert "case_id" in fields
    assert "fixture_file" in fields


def test_explicit_order_difference_is_still_a_divergence(differ):
    """Ordering IS part of the contract when both sides claim one."""
    pinned = {"index": [], "column_order": ["a", "b"], "columns": {"a": [], "b": []}}
    live = {"index": [], "column_order": ["b", "a"], "columns": {"a": [], "b": []}}
    equal, message = differ.frame_equal(pinned, live)
    assert not equal
    assert "column_order mismatch" in message


def test_error_expecting_fixture_counts_as_derivable_and_matching(differ, monkeypatch):
    """An oracle error is the BEHAVIOUR UNDER TEST for these fixtures.

    br-frankenpandas-fixture-divergence-triage-9s0c4. Two bugs combined to hide
    the entire error surface: `run_live_oracle` discarded the oracle's response
    whenever it exited non-zero (it exits 1 on OracleError but still writes a
    well-formed JSON body carrying the message), and `diff_packet` then treated
    that as "failed to run". So a fixture pinning an expected error was reported
    as non-derivable and its error path was never compared at all.
    """
    monkeypatch.setattr(
        differ, "run_live_oracle", lambda *a, **k: {"error": "unsupported constructor dtype 'uint64'"}
    )
    packet = {
        "packet_id": "X",
        "case_id": "x",
        "operation": "dataframe_constructor_list_like",
        "expected_error_contains": "unsupported constructor dtype 'uint64'",
    }
    result = differ.diff_packet(packet, "/nonexistent", "oracle.py", fixture_file="x.json")
    assert result.live_derivable
    assert result.matches_pinned
    assert result.divergence == ""


def test_error_wording_difference_still_matches_but_is_noted(differ, monkeypatch):
    """DISC-003: conformance checks the error CATEGORY, not the exact string."""
    monkeypatch.setattr(
        differ, "run_live_oracle", lambda *a, **k: {"error": "Trying to coerce float values to integers"}
    )
    packet = {
        "packet_id": "X",
        "case_id": "x",
        "operation": "dataframe_constructor_list_like",
        "expected_error_contains": "cannot cast float 1.5 to int64 without loss",
    }
    result = differ.diff_packet(packet, "/nonexistent", "oracle.py", fixture_file="x.json")
    assert result.matches_pinned
    assert "DISC-003" in result.divergence


def test_unexpected_oracle_error_is_still_non_derivable(differ, monkeypatch):
    """The negative control: a VALUE fixture that errors is not a pass.

    Without this, the change above would turn every oracle failure into a
    silent success — the exact shape of bug this whole bead is about.
    """
    monkeypatch.setattr(differ, "run_live_oracle", lambda *a, **k: {"error": "boom"})
    packet = {
        "packet_id": "X",
        "case_id": "x",
        "operation": "series_head",
        "expected_series": {"index": [], "values": []},
    }
    result = differ.diff_packet(packet, "/nonexistent", "oracle.py", fixture_file="x.json")
    assert not result.live_derivable
    assert not result.matches_pinned
