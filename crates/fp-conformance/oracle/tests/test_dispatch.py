"""Dispatch tests for pandas_oracle.py.

Per br-frankenpandas-urhy: exercise a handful of canonical op handlers
end-to-end through `dispatch()` to confirm the payload-to-response
contract stays green as handlers evolve.
"""
from __future__ import annotations

import inspect
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


@pytest.mark.parametrize(
    ("operation", "other", "expected"),
    [
        ("dataframe_where", -1, {"a": [1, -1], "b": [-1, -1]}),
        ("dataframe_mask", -1, {"a": [-1, 2], "b": [-1, -1]}),
        ("dataframe_where_df", None, {"a": [1, 20], "b": [30, 40]}),
        ("dataframe_mask_df", None, {"a": [10, 2], "b": [30, 40]}),
    ],
)
def test_dataframe_where_and_mask_align_partial_condition_columns(
    oracle, pd, operation, other, expected
):
    payload = {
        "operation": operation,
        "frame": _frame_payload({"a": [1, 2], "b": [3, 4]}),
        "frame_right": {
            "index": [
                {"kind": "int64", "value": 0},
                {"kind": "int64", "value": 1},
            ],
            "columns": {
                "a": [
                    {"kind": "bool", "value": True},
                    {"kind": "bool", "value": False},
                ]
            },
        },
    }
    if other is None:
        payload["frame_other"] = _frame_payload({"a": [10, 20], "b": [30, 40]})
    else:
        payload["fill_value"] = {"kind": "int64", "value": other}

    response = oracle.dispatch(pd, payload)
    columns = response["expected_frame"]["columns"]
    assert {name: [item["value"] for item in values] for name, values in columns.items()} == expected


def test_dataframe_set_index_missing_column_is_a_pandas_error(oracle, pd):
    payload = {
        "operation": "dataframe_set_index",
        "frame": _frame_payload({"present": [1, 2]}),
        "set_index_column": "missing",
        "set_index_drop": True,
    }

    with pytest.raises(oracle.OracleError) as exc_info:
        oracle.dispatch(pd, payload)

    assert str(exc_info.value) == (
        "dataframe_set_index failed: \"None of ['missing'] are in the columns\""
    )
    assert oracle.oracle_error_origin(exc_info.value) == oracle.ERROR_ORIGIN_PANDAS


def test_dataframe_concat_invalid_axis_is_a_pandas_error(oracle, pd):
    payload = {
        "operation": "dataframe_concat",
        "frame": _frame_payload({"left": [1]}),
        "frame_right": _frame_payload({"right": [2]}),
        "concat_axis": 2,
    }

    with pytest.raises(oracle.OracleError) as exc_info:
        oracle.dispatch(pd, payload)

    assert str(exc_info.value) == (
        "dataframe_concat failed: No axis named 2 for object type DataFrame"
    )
    assert oracle.oracle_error_origin(exc_info.value) == oracle.ERROR_ORIGIN_PANDAS


def test_dataframe_concat_invalid_join_is_a_pandas_error(oracle, pd):
    payload = {
        "operation": "dataframe_concat",
        "frame": _frame_payload({"left": [1]}),
        "frame_right": _frame_payload({"right": [2]}),
        "concat_join": "sideways",
    }

    with pytest.raises(oracle.OracleError) as exc_info:
        oracle.dispatch(pd, payload)

    assert str(exc_info.value) == (
        "dataframe_concat failed: Only can inner (intersect) or outer (union) "
        "join the other axis"
    )
    assert oracle.oracle_error_origin(exc_info.value) == oracle.ERROR_ORIGIN_PANDAS


def test_series_join_cross_is_a_pandas_error(oracle, pd):
    payload = {
        "operation": "series_join",
        "left": _series_payload([1], [0]),
        "right": _series_payload([2], [0]),
        "join_type": "cross",
    }

    with pytest.raises(oracle.OracleError) as exc_info:
        oracle.dispatch(pd, payload)

    assert str(exc_info.value) == (
        "series_join failed: Can not pass on, right_on, left_on or set "
        "right_index=True or left_index=True"
    )
    assert oracle.oracle_error_origin(exc_info.value) == oracle.ERROR_ORIGIN_PANDAS


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


def test_series_str_rsplit_get_preserves_supplied_object_missing_value(oracle, pd):
    payload = {
        "operation": "series_str_rsplit_get",
        "str_split_pat": "/",
        "str_split_n": 0,
        "left": {
            "index": [{"kind": "int64", "value": i} for i in range(4)],
            "values": [
                {"kind": "utf8", "value": "a/b/c"},
                {"kind": "null", "value": "null"},
                {"kind": "utf8", "value": ""},
                {"kind": "utf8", "value": "no_slash"},
            ],
        },
    }

    response = oracle.dispatch(pd, payload)

    assert response["expected_series"]["values"] == [
        {"kind": "utf8", "value": "a"},
        {"kind": "null", "value": "null"},
        {"kind": "utf8", "value": ""},
        {"kind": "utf8", "value": "no_slash"},
    ]


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


def _datetime_series_payload(values):
    return {
        "index": [{"kind": "int64", "value": i} for i, _ in enumerate(values)],
        "values": [{"kind": "utf8", "value": value} for value in values],
    }


@pytest.mark.parametrize(
    ("operation", "expected_kind"),
    [
        ("series_dt_year", "int64"),
        ("series_dt_month", "int64"),
        ("series_dt_day", "int64"),
        ("series_dt_hour", "int64"),
        ("series_dt_minute", "int64"),
        ("series_dt_second", "int64"),
        ("series_dt_dayofweek", "int64"),
        ("series_dt_dayofyear", "int64"),
        ("series_dt_quarter", "int64"),
        ("series_dt_is_month_start", "bool"),
        ("series_dt_is_month_end", "bool"),
        ("series_dt_is_year_start", "bool"),
        ("series_dt_is_year_end", "bool"),
        ("series_dt_is_leap_year", "bool"),
    ],
)
def test_series_dt_accessors_dispatch(oracle, pd, operation, expected_kind):
    payload = {
        "operation": operation,
        "left": _datetime_series_payload(["2024-01-15", "2024-06-30", "2024-12-31"]),
    }
    response = oracle.dispatch(pd, payload)
    assert "expected_series" in response
    values = response["expected_series"]["values"]
    assert len(values) == 3
    assert all(v["kind"] in (expected_kind, "null") for v in values)


def test_series_dt_day_name_returns_strings(oracle, pd):
    payload = {
        "operation": "series_dt_day_name",
        "left": _datetime_series_payload(["2024-01-01", "2024-01-02"]),
    }
    response = oracle.dispatch(pd, payload)
    values = [v["value"] for v in response["expected_series"]["values"]]
    assert values == ["Monday", "Tuesday"]


def test_series_dt_month_name_returns_strings(oracle, pd):
    payload = {
        "operation": "series_dt_month_name",
        "left": _datetime_series_payload(["2024-01-15", "2024-06-15"]),
    }
    response = oracle.dispatch(pd, payload)
    values = [v["value"] for v in response["expected_series"]["values"]]
    assert values == ["January", "June"]


def test_series_concat_combines_series(oracle, pd):
    payload = {
        "operation": "series_concat",
        "left": _series_payload([1, 2], [0, 1]),
        "right": _series_payload([3, 4], [2, 3]),
    }
    response = oracle.dispatch(pd, payload)
    assert "expected_series" in response
    values = [v["value"] for v in response["expected_series"]["values"]]
    assert len(values) == 4


def test_series_to_timedelta_converts_to_timedelta(oracle, pd):
    payload = {
        "operation": "series_to_timedelta",
        "left": _utf8_series_payload(["1 days", "2 hours"]),
    }
    response = oracle.dispatch(pd, payload)
    assert "expected_series" in response


@pytest.mark.parametrize(
    ("operation", "expected"),
    [
        ("series_str_casefold", ["abc", "hello"]),
        ("series_str_isdecimal", [False, True]),
        ("series_str_istitle", [True, False]),
    ],
)
def test_series_str_new_unary_dispatches(oracle, pd, operation, expected):
    inputs = ["ABC", "hello"] if operation == "series_str_casefold" else (
        ["abc", "123"] if operation == "series_str_isdecimal" else ["Hello World", "hello"]
    )
    payload = {
        "operation": operation,
        "left": _utf8_series_payload(inputs),
    }
    response = oracle.dispatch(pd, payload)
    assert _expected_values(response) == expected


def test_series_str_normalize_nfc(oracle, pd):
    payload = {
        "operation": "series_str_normalize",
        "left": _utf8_series_payload(["café"]),
        "str_normalize_form": "NFC",
    }
    response = oracle.dispatch(pd, payload)
    assert "expected_series" in response


def test_series_str_get_extracts_character(oracle, pd):
    payload = {
        "operation": "series_str_get",
        "left": _utf8_series_payload(["abc", "xyz"]),
        "str_get_index": 0,
    }
    response = oracle.dispatch(pd, payload)
    values = _expected_values(response)
    assert values == ["a", "x"]


def test_groupby_std_renders_an_undefined_group_as_nan_not_none(oracle, pd):
    """A one-member group's std is a float NaN, and the oracle must say so.

    br-frankenpandas-fixture-divergence-triage-9s0c4. `op_groupby_agg` used to
    rewrite a std/var NaN into {"kind": "null", "value": "null"}, commented
    "Runtime currently models n<2 std/var as null (not NaN) for parity" — the
    oracle bent to FrankenPandas and then banked as truth. It was stale too: FP
    emits Null(NullKind::NaN) for n <= 1.

    MEASURED, live pandas 2.2.3, key=['a',None,'a','b','b'] and
    value=[10,20,nan,40,50] so that group 'a' has ONE present value:

        df.groupby('key')['value'].std()
          -> {'a': nan, 'b': 7.0710678118654755}   dtype float64

    The four fp_p2c_011 fixtures already pinned na_n; only the oracle disagreed.
    """
    payload = {
        "operation": "groupby_std",
        "left": {
            "name": "key",
            "index": [{"kind": "int64", "value": i} for i in range(5)],
            "values": [
                {"kind": "utf8", "value": "a"},
                {"kind": "null", "value": "null"},
                {"kind": "utf8", "value": "a"},
                {"kind": "utf8", "value": "b"},
                {"kind": "utf8", "value": "b"},
            ],
        },
        "right": {
            "name": "value",
            "index": [{"kind": "int64", "value": i} for i in range(5)],
            "values": [
                {"kind": "int64", "value": 10},
                {"kind": "int64", "value": 20},
                {"kind": "null", "value": "na_n"},
                {"kind": "int64", "value": 40},
                {"kind": "int64", "value": 50},
            ],
        },
    }
    values = oracle.dispatch(pd, payload)["expected_series"]["values"]
    assert values[0] == {"kind": "null", "value": "na_n"}, (
        "an undefined group std is a float NaN; rewriting it to a generic null "
        "adapts the oracle to FrankenPandas instead of measuring pandas"
    )
    assert values[1]["kind"] == "float64"

    payload["operation"] = "groupby_var"
    assert oracle.dispatch(pd, payload)["expected_series"]["values"][0] == {
        "kind": "null",
        "value": "na_n",
    }


def test_groupby_ngroup_honours_sort_ascending(oracle, pd):
    """The descending numbering must reach pandas, not be silently defaulted.

    br-frankenpandas-fixture-divergence-triage-9s0c4. op_dataframe_groupby_ngroup
    called .ngroup() with no ascending argument, so a fixture carrying
    sort_ascending=False exercised pandas' ascending DEFAULT instead.

    MEASURED, live pandas 2.2.3, on grp = ['a','b','a','c','b','a']:
        .ngroup()                -> [0, 1, 0, 2, 1, 0]
        .ngroup(ascending=False) -> [2, 1, 2, 0, 1, 2]

    test_payload_keys_are_read.py cannot catch this and says so in its own
    docstring: sort_ascending IS read elsewhere in the oracle, so the
    key-appears check passes while this one handler ignores it.
    """
    frame = {
        "index": [{"kind": "int64", "value": i} for i in range(6)],
        "column_order": ["grp", "val"],
        "columns": {
            "grp": [
                {"kind": "utf8", "value": v} for v in ["a", "b", "a", "c", "b", "a"]
            ],
            "val": [
                {"kind": "int64", "value": v} for v in [10, 20, 30, 40, 50, 60]
            ],
        },
    }
    base = {
        "operation": "dataframe_groupby_ngroup",
        "frame": frame,
        "groupby_columns": ["grp"],
    }

    def values(payload):
        out = oracle.dispatch(pd, payload)["expected_series"]["values"]
        return [v["value"] for v in out]

    assert values(base) == [0, 1, 0, 2, 1, 0], "no key supplied -> pandas' default"
    assert values({**base, "sort_ascending": True}) == [0, 1, 0, 2, 1, 0]
    assert values({**base, "sort_ascending": False}) == [2, 1, 2, 0, 1, 2], (
        "sort_ascending=False must reach pandas as ngroup(ascending=False)"
    )

    with pytest.raises(oracle.OracleError, match="must be a bool"):
        oracle.dispatch(pd, {**base, "sort_ascending": "no"})


def test_from_dict_honours_constructor_dtype(oracle, pd):
    """dataframe_from_dict never read constructor_dtype.

    Third member of the family resolve_constructor_dtype's docstring describes
    as fixed, after from_series (51fb88ead).

    MEASURED, live pandas 2.2.3:
        pd.DataFrame({'a':[1,2],'b':[3,4]})                  -> int64
        pd.DataFrame({'a':[1,2],'b':[3,4]}, dtype='float64') -> float64

    fp_p2d_023_dataframe_from_dict_dtype_float64_strict pins the float64 and was
    RIGHT; the oracle returned int64.
    """
    payload = {
        "operation": "dataframe_from_dict",
        "dict_columns": {
            "a": [{"kind": "int64", "value": 1}, {"kind": "int64", "value": 2}],
            "b": [{"kind": "int64", "value": 3}, {"kind": "int64", "value": 4}],
        },
    }

    def kinds(p):
        cols = oracle.dispatch(pd, p)["expected_frame"]["columns"]
        return {name: values[0]["kind"] for name, values in cols.items()}

    assert kinds(payload) == {"a": "int64", "b": "int64"}, "no key -> pandas' inference"
    assert kinds({**payload, "constructor_dtype": "float64"}) == {
        "a": "float64",
        "b": "float64",
    }
    # The alias/trim normalization resolve_constructor_dtype exists for.
    assert kinds({**payload, "constructor_dtype": "  F64 "}) == {
        "a": "float64",
        "b": "float64",
    }


def test_series_split_df_honours_str_split_n(oracle, pd):
    """series_split_df never read str_split_n.

    Same dropped-argument family as groupby ngroup's sort_ascending
    (905ba264e) and from_dict's constructor_dtype (03e6dd575): the fixture
    stores the option, the Rust harness passes it
    (Series::str().split_expand_n), and the oracle silently answered a
    DIFFERENT question -- an unlimited split.

    MEASURED, live pandas 2.2.3, pd.Series(['a_b_c','d_e','f']):
        .str.split('_', n=1,  expand=True) -> 2 columns  [['a','b_c'],['d','e'],['f',None]]
        .str.split('_', n=0,  expand=True) -> 3 columns  (n <= 0 is UNLIMITED)
        .str.split('_', n=-1, expand=True) -> 3 columns  (pandas' own default)
        .str.split('_',       expand=True) -> 3 columns

    fp_p2d_431_series_str_split_expand_n_padding_strict pins n=1 and the two
    columns, and was RIGHT; the oracle returned three.
    """
    payload = {
        "operation": "series_split_df",
        "str_split_pat": "_",
        "left": {
            "name": "parts",
            "index": [{"kind": "int64", "value": i} for i in range(3)],
            "values": [
                {"kind": "utf8", "value": "a_b_c"},
                {"kind": "utf8", "value": "d_e"},
                {"kind": "utf8", "value": "f"},
            ],
        },
    }

    def order(p):
        return oracle.dispatch(pd, p)["expected_frame"]["column_order"]

    assert order(payload) == ["0", "1", "2"], "no key -> pandas' default (unlimited)"
    assert order({**payload, "str_split_n": 1}) == ["0", "1"], (
        "str_split_n=1 must reach pandas as str.split(n=1)"
    )
    # Non-positive n is unlimited, so it must agree with the absent key.
    assert order({**payload, "str_split_n": 0}) == ["0", "1", "2"]
    assert order({**payload, "str_split_n": -1}) == ["0", "1", "2"]

    with pytest.raises(oracle.OracleError, match="must be an integer"):
        oracle.dispatch(pd, {**payload, "str_split_n": "1"})


def test_series_dt_date_encodes_nat_as_null_not_the_string_nat(oracle, pd):
    """series_dt_date hand-rolled its encoder and leaked the string "NaT".

    The private copy was
        {"kind": "utf8", "value": str(v)} if v is not None else <null>
    and `.dt.date` puts NaT — not None — in a missing slot, so the guard held
    and a MISSING value was banked as a real utf8 value "NaT".

    MEASURED, live pandas 2.2.3:
        pd.to_datetime(pd.Series(['2024-03-15T23:59:59', None])).dt.date
          -> dtype object, [datetime.date(2024, 3, 15), NaT]
        type(pd.NaT).__name__ == 'NaTType'   pd.NaT is None -> False
        pd.isna(pd.NaT) -> True

    fp_p2d_322_series_dt_date_null_hardened pins null at that row and was RIGHT;
    the oracle returned {"kind": "utf8", "value": "NaT"}.

    Same shape as br-frankenpandas-oxodo: a private re-implementation of
    something the shared helper already did correctly. Every sibling dt accessor
    calls series_to_expected; this one did not.
    """
    payload = {
        "operation": "series_dt_date",
        "left": {
            "name": "datetimes",
            "index": [{"kind": "int64", "value": i} for i in range(4)],
            "values": [
                {"kind": "utf8", "value": "2024-03-15T23:59:59"},
                {"kind": "null", "value": "null"},
                {"kind": "utf8", "value": "2024-07-04T00:00:01"},
                {"kind": "utf8", "value": "not a date at all"},
            ],
        },
    }

    values = oracle.dispatch(pd, payload)["expected_series"]["values"]
    assert values[0] == {"kind": "utf8", "value": "2024-03-15"}
    assert values[2] == {"kind": "utf8", "value": "2024-07-04"}
    # The supplied null, and the unparseable string that errors="coerce" turns
    # into NaT, must BOTH be missing — neither may be the text "NaT".
    assert values[1] == {"kind": "null", "value": "null"}, "supplied null leaked"
    assert values[3] == {"kind": "null", "value": "null"}, "coerced NaT leaked"
    assert not any(
        v.get("value") == "NaT" for v in values
    ), f"a missing value was encoded as the string 'NaT': {values}"


def test_from_records_honours_constructor_dtype(oracle, pd):
    """dataframe_from_records never read constructor_dtype.

    Fourth and last member of the family resolve_constructor_dtype's docstring
    describes, after from_series (51fb88ead) and from_dict (03e6dd575).

    DataFrame.from_records has NO dtype= parameter -- its signature is
    (data, index, exclude, columns, coerce_float, nrows) -- so the oracle now
    routes through pd.DataFrame(...) when, and only when, a dtype is asked for.

    MEASURED, live pandas 2.2.3:
        pd.DataFrame([{'a':1.0,'b':True},{'a':2.0,'b':False}])
          -> a float64 [1.0, 2.0], b bool [True, False]
        pd.DataFrame([{'a':1.0,'b':True},{'a':2.0,'b':False}], dtype='int64')
          -> a int64 [1, 2], b int64 [1, 0]
        pd.DataFrame([{'a':1}], dtype='string') -> a ['1'] as python str

    fp_p2d_023_dataframe_from_records_dtype_int64_strict and
    ..._dtype_utf8_coerced_hardened pin those and were RIGHT.
    """
    assert "dtype" not in inspect.signature(pd.DataFrame.from_records).parameters, (
        "from_records grew a dtype= parameter; the pd.DataFrame() detour may no "
        "longer be needed"
    )

    payload = {
        "operation": "dataframe_from_records",
        "records": [
            {"a": {"kind": "float64", "value": 1.0}, "b": {"kind": "bool", "value": True}},
            {"a": {"kind": "float64", "value": 2.0}, "b": {"kind": "bool", "value": False}},
        ],
    }

    def cols(p):
        return oracle.dispatch(pd, p)["expected_frame"]["columns"]

    plain = cols(payload)
    assert [v["kind"] for v in plain["a"]] == ["float64", "float64"], "no key -> pandas' inference"
    assert [v["kind"] for v in plain["b"]] == ["bool", "bool"]

    typed = cols({**payload, "constructor_dtype": "int64"})
    assert [(v["kind"], v["value"]) for v in typed["a"]] == [("int64", 1), ("int64", 2)]
    # The bool column casts to 1/0 under an int64 constructor dtype.
    assert [(v["kind"], v["value"]) for v in typed["b"]] == [("int64", 1), ("int64", 0)]

    # utf8 normalizes to pandas' "string" and renders the int as text.
    coerced = cols({
        "operation": "dataframe_from_records",
        "records": [{"a": {"kind": "int64", "value": 1}}],
        "constructor_dtype": "utf8",
    })
    assert coerced["a"] == [{"kind": "utf8", "value": "1"}]

    with pytest.raises(oracle.OracleError, match="unsupported constructor dtype"):
        oracle.dispatch(pd, {**payload, "constructor_dtype": "no-such-dtype"})


def test_from_records_without_dtype_still_uses_from_records(oracle, pd):
    """The no-dtype path must NOT be rerouted through pd.DataFrame().

    from_records(index=...) can name a COLUMN to index by; pd.DataFrame(index=...)
    reads the same argument as LABELS. 11 of the 13 from_records fixtures use the
    no-dtype path, so the detour is gated on a dtype actually being requested.

    MEASURED, live pandas 2.2.3:
        pd.DataFrame.from_records([{'k':'x','v':1},{'k':'y','v':2}], index='k')
          -> index Index(['x','y'], name='k'), one column v
        pd.DataFrame([{'k':'x','v':1},{'k':'y','v':2}], index='k')
          -> raises; 'k' is not a valid list of labels
    """
    payload = {
        "operation": "dataframe_from_records",
        "records": [
            {"k": {"kind": "utf8", "value": "x"}, "v": {"kind": "int64", "value": 1}},
            {"k": {"kind": "utf8", "value": "y"}, "v": {"kind": "int64", "value": 2}},
        ],
    }
    frame = oracle.dispatch(pd, payload)["expected_frame"]
    assert [v["value"] for v in frame["columns"]["k"]] == ["x", "y"]
    assert [v["value"] for v in frame["columns"]["v"]] == [1, 2]
    assert [v["value"] for v in frame["index"]] == [0, 1]
