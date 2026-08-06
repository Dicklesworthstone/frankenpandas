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


def test_explicit_order_difference_is_still_a_divergence(differ):
    """Ordering IS part of the contract when both sides claim one."""
    pinned = {"index": [], "column_order": ["a", "b"], "columns": {"a": [], "b": []}}
    live = {"index": [], "column_order": ["b", "a"], "columns": {"a": [], "b": []}}
    equal, message = differ.frame_equal(pinned, live)
    assert not equal
    assert "column_order mismatch" in message
