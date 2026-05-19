"""Dispatch tests for pandas_oracle.py.

Per br-frankenpandas-urhy: exercise a handful of canonical op handlers
end-to-end through `dispatch()` to confirm the payload-to-response
contract stays green as handlers evolve.
"""
from __future__ import annotations

import sys
from types import SimpleNamespace

import pytest


def _series_payload(values, index):
    return {
        "index": [{"kind": "int64", "value": int(i)} for i in index],
        "values": [{"kind": "int64", "value": int(v)} for v in values],
    }


def _utf8_series_payload(values):
    return {
        "index": [{"kind": "int64", "value": i} for i, _ in enumerate(values)],
        "values": [{"kind": "utf8", "value": value} for value in values],
    }


def _frame_payload(columns):
    first_column = next(iter(columns.values()))
    return {
        "index": [
            {"kind": "int64", "value": i} for i, _ in enumerate(first_column)
        ],
        "columns": {
            name: [{"kind": "int64", "value": int(v)} for v in values]
            for name, values in columns.items()
        },
    }


def _expected_values(response):
    return [item["value"] for item in response["expected_series"]["values"]]


def test_series_add_produces_index_aligned_sum(oracle, pd):
    payload = {
        "operation": "series_add",
        "left": _series_payload([1, 2, 3], [0, 1, 2]),
        "right": _series_payload([10, 20, 30], [0, 1, 2]),
    }
    response = oracle.dispatch(pd, payload)
    assert "expected_series" in response
    values = [v["value"] for v in response["expected_series"]["values"]]
    assert values == [11.0, 22.0, 33.0]


def test_series_sub_aligns_and_subtracts(oracle, pd):
    payload = {
        "operation": "series_sub",
        "left": _series_payload([10, 20], [0, 1]),
        "right": _series_payload([1, 2], [0, 1]),
    }
    response = oracle.dispatch(pd, payload)
    values = [v["value"] for v in response["expected_series"]["values"]]
    assert values == [9.0, 18.0]


def test_series_nunique_counts_distinct(oracle, pd):
    payload = {
        "operation": "series_nunique",
        "series": _series_payload([1, 1, 2, 3, 3], [0, 1, 2, 3, 4]),
    }
    response = oracle.dispatch(pd, payload)
    assert response["expected_scalar"]["kind"] == "int64"
    assert response["expected_scalar"]["value"] == 3


def test_dataframe_cumsum_preserves_integer_dtype(oracle, pd):
    payload = {
        "operation": "dataframe_cumsum",
        "frame": _frame_payload({"value": [1, 2, 3]}),
    }
    response = oracle.dispatch(pd, payload)
    values = response["expected_frame"]["columns"]["value"]
    assert [item["kind"] for item in values] == ["int64", "int64", "int64"]
    assert [item["value"] for item in values] == [1, 3, 6]


def test_groupby_min_preserves_integer_dtype(oracle, pd):
    payload = {
        "operation": "groupby_min",
        "left": {
            "index": [{"kind": "int64", "value": i} for i in range(4)],
            "values": [
                {"kind": "utf8", "value": value}
                for value in ["a", "b", "a", "b"]
            ],
        },
        "right": _series_payload([10, 20, 30, 15], [0, 1, 2, 3]),
    }
    response = oracle.dispatch(pd, payload)
    values = response["expected_series"]["values"]
    assert [item["kind"] for item in values] == ["int64", "int64"]
    assert [item["value"] for item in values] == [10, 15]


def test_groupby_first_encodes_nullable_integer_missing_values(oracle, pd):
    payload = {
        "operation": "groupby_first",
        "left": {
            "index": [{"kind": "int64", "value": i} for i in range(3)],
            "values": [
                {"kind": "utf8", "value": value} for value in ["x", "y", "z"]
            ],
        },
        "right": {
            "index": [{"kind": "int64", "value": i} for i in range(3)],
            "values": [
                {"kind": "null", "value": "null"},
                {"kind": "int64", "value": 2},
                {"kind": "null", "value": "null"},
            ],
        },
    }
    response = oracle.dispatch(pd, payload)
    values = response["expected_series"]["values"]
    assert values == [
        {"kind": "null", "value": "null"},
        {"kind": "int64", "value": 2},
        {"kind": "null", "value": "null"},
    ]


def test_groupby_min_encodes_nan_marker_nullable_integer_missing_values(oracle, pd):
    payload = {
        "operation": "groupby_min",
        "left": {
            "index": [{"kind": "int64", "value": i} for i in range(3)],
            "values": [
                {"kind": "utf8", "value": value} for value in ["x", "x", "y"]
            ],
        },
        "right": {
            "index": [{"kind": "int64", "value": i} for i in range(3)],
            "values": [
                {"kind": "int64", "value": 10},
                {"kind": "null", "value": "na_n"},
                {"kind": "int64", "value": 3},
            ],
        },
    }
    response = oracle.dispatch(pd, payload)
    values = response["expected_series"]["values"]
    assert values == [
        {"kind": "int64", "value": 10},
        {"kind": "int64", "value": 3},
    ]


def test_series_abs_encodes_nullable_integer_missing_values(oracle, pd):
    payload = {
        "operation": "series_abs",
        "left": {
            "index": [{"kind": "int64", "value": i} for i in range(3)],
            "values": [
                {"kind": "int64", "value": -7},
                {"kind": "null", "value": "na_n"},
                {"kind": "int64", "value": 4},
            ],
        },
    }
    response = oracle.dispatch(pd, payload)
    values = response["expected_series"]["values"]
    assert values == [
        {"kind": "int64", "value": 7},
        {"kind": "null", "value": "null"},
        {"kind": "int64", "value": 4},
    ]


def test_dataframe_abs_encodes_nullable_integer_columns(oracle, pd):
    payload = {
        "operation": "dataframe_abs",
        "frame": {
            "index": [{"kind": "int64", "value": i} for i in range(3)],
            "columns": {
                "nums": [
                    {"kind": "int64", "value": -7},
                    {"kind": "null", "value": "na_n"},
                    {"kind": "int64", "value": 4},
                ]
            },
        },
    }
    response = oracle.dispatch(pd, payload)
    values = response["expected_frame"]["columns"]["nums"]
    assert values == [
        {"kind": "int64", "value": 7},
        {"kind": "null", "value": "null"},
        {"kind": "int64", "value": 4},
    ]


@pytest.mark.parametrize(
    ("operation", "expected"),
    [
        ("series_str_swapcase", ["aBc", "HELLO", "123", " ", ""]),
        ("series_str_isdigit", [False, False, True, False, False]),
        ("series_str_isalpha", [True, True, False, False, False]),
        ("series_str_isalnum", [True, True, True, False, False]),
        ("series_str_isspace", [False, False, False, True, False]),
        ("series_str_islower", [False, True, False, False, False]),
        ("series_str_isupper", [False, False, False, False, False]),
        ("series_str_isnumeric", [False, False, True, False, False]),
    ],
)
def test_series_str_unary_dispatches_to_pandas(oracle, pd, operation, expected):
    payload = {
        "operation": operation,
        "left": _utf8_series_payload(["AbC", "hello", "123", " ", ""]),
    }
    response = oracle.dispatch(pd, payload)
    assert _expected_values(response) == expected


@pytest.mark.parametrize(
    ("operation", "extra", "expected"),
    [
        ("series_str_contains", {"regex_pattern": "a"}, [True, True, False]),
        ("series_str_startswith", {"regex_pattern": "a"}, [True, False, False]),
        ("series_str_endswith", {"regex_pattern": "a"}, [True, True, False]),
        (
            "series_str_replace",
            {"regex_pattern": "a", "replace_value": "X"},
            ["XlphX", "betX", "end"],
        ),
    ],
)
def test_series_str_pattern_dispatches_to_pandas(oracle, pd, operation, extra, expected):
    payload = {
        "operation": operation,
        "left": _utf8_series_payload(["alpha", "beta", "end"]),
        **extra,
    }
    response = oracle.dispatch(pd, payload)
    assert _expected_values(response) == expected


@pytest.mark.parametrize(
    ("operation", "extra", "expected"),
    [
        ("series_str_center", {}, ["--a--", "-abc-"]),
        ("series_str_ljust", {}, ["a----", "abc--"]),
        ("series_str_rjust", {}, ["----a", "--abc"]),
        ("series_str_pad", {"str_pad_side": "both"}, ["--a--", "-abc-"]),
        ("series_str_pad", {"str_pad_side": "left"}, ["----a", "--abc"]),
        ("series_str_pad", {"str_pad_side": "right"}, ["a----", "abc--"]),
    ],
)
def test_series_str_padding_dispatches_to_pandas(oracle, pd, operation, extra, expected):
    payload = {
        "operation": operation,
        "left": _utf8_series_payload(["a", "abc"]),
        "str_width": 5,
        "str_fillchar": "-",
        **extra,
    }
    response = oracle.dispatch(pd, payload)
    assert _expected_values(response) == expected


def test_dispatch_rejects_unknown_operation(oracle, pd):
    with pytest.raises(oracle.OracleError):
        oracle.dispatch(pd, {"operation": "operation_that_does_not_exist"})


def test_dispatch_requires_operation_key(oracle, pd):
    with pytest.raises((oracle.OracleError, KeyError, TypeError)):
        oracle.dispatch(pd, {})


def test_series_add_requires_both_sides(oracle, pd):
    payload = {
        "operation": "series_add",
        "left": _series_payload([1], [0]),
        # right missing
    }
    with pytest.raises(oracle.OracleError):
        oracle.dispatch(pd, payload)


def test_setup_pandas_strict_legacy_rejects_system_import(oracle, tmp_path):
    args = SimpleNamespace(
        legacy_root=str(tmp_path / "pandas"),
        strict_legacy=True,
        allow_system_pandas_fallback=False,
    )
    original_path = list(sys.path)
    try:
        with pytest.raises(oracle.OracleError, match="outside legacy root"):
            oracle.setup_pandas(args)
    finally:
        sys.path[:] = original_path


def test_setup_pandas_strict_legacy_allows_system_fallback(oracle, tmp_path):
    args = SimpleNamespace(
        legacy_root=str(tmp_path / "pandas"),
        strict_legacy=True,
        allow_system_pandas_fallback=True,
    )
    original_path = list(sys.path)
    try:
        pd = oracle.setup_pandas(args)
    finally:
        sys.path[:] = original_path
    assert hasattr(pd, "Series")
