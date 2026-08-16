"""Dispatch tests for pandas_oracle.py.

Per br-frankenpandas-urhy: exercise a handful of canonical op handlers
end-to-end through `dispatch()` to confirm the payload-to-response
contract stays green as handlers evolve.
"""
from __future__ import annotations

import copy
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


def test_csv_read_frame_invalid_on_bad_lines_is_a_pandas_error(oracle, pd):
    payload = {
        "operation": "csv_read_frame",
        "csv_input": "a,b\n1,2\n",
        "csv_on_bad_lines": "bogus",
    }

    with pytest.raises(oracle.OracleError) as exc_info:
        oracle.dispatch(pd, payload)

    assert str(exc_info.value) == (
        "csv_read_frame failed: Argument bogus is invalid for on_bad_lines"
    )
    assert oracle.oracle_error_origin(exc_info.value) == oracle.ERROR_ORIGIN_PANDAS


def test_series_between_invalid_inclusive_is_a_pandas_error(oracle, pd):
    payload = {
        "operation": "series_between",
        "left": _series_payload([1, 2, 3], [0, 1, 2]),
        "between_left": {"kind": "int64", "value": 1},
        "between_right": {"kind": "int64", "value": 3},
        "between_inclusive": "bogus",
    }

    with pytest.raises(oracle.OracleError) as exc_info:
        oracle.dispatch(pd, payload)

    assert str(exc_info.value) == (
        "series_between failed: Inclusive has to be either string of "
        "'both','left', 'right', or 'neither'."
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


def test_groupby_std_undefined_group_missing_kind_follows_the_payload_dtype(oracle, pd):
    """The PAYLOAD's dtype decides the missing KIND of a derived float aggregate.

    br-frankenpandas-fixture-divergence-triage-9s0c4 wrote the original of this
    test to stop `op_groupby_agg` rewriting a std/var NaN into a generic null,
    commented "Runtime currently models n<2 std/var as null (not NaN) for
    parity" — the oracle bending to FrankenPandas and banking it as truth. That
    concern is still live and is the FLOAT64 half below.

    What changed is the premise, not the principle. That test measured a lane the
    oracle then forced to float64 for every agg; br-frankenpandas-778bb removed
    the forcing, so an int+null payload now builds the NULLABLE Int64 the corpus
    means by it (DISCREPANCIES DISC-011), and pandas answers such a lane in its
    Float64 EXTENSION, whose missing is pd.NA rather than a float NaN.

    MEASURED, live pandas 2.2.3, key=['a',None,'a','b','b'] and
    value=[10,20,<missing>,40,50] so group 'a' has ONE present value:

        value float64 -> std dtype float64, group 'a' = np.float64(nan)
        value Int64   -> std dtype Float64, group 'a' = <NA>   (NAType)

    Both are pandas' real answers; they differ only in how the LANE was built.
    The fixture format already distinguishes them — `na_n` is a float NaN and
    `null` is pd.NA — so no format change is needed to record either, which is
    the question br-frankenpandas-qcvzc was filed to decide.

    BOTH ARMS ARE LOAD-BEARING. Keep only the Int64 one and the oracle may go
    back to rewriting a genuine float NaN into a generic null (the original
    defect). Keep only the float64 one and the oracle must mis-report pd.NA as a
    float NaN, which is what this test asserted after 778bb landed and is why
    CI's oracle-pytest step was red. (br-frankenpandas-qcvzc)
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
    # INT + NULL payload -> nullable Int64 lane -> Float64 extension -> pd.NA.
    values = oracle.dispatch(pd, payload)["expected_series"]["values"]
    assert values[0] == {"kind": "null", "value": "null"}, (
        "a nullable-Int64 lane answers std in pandas' Float64 extension, whose "
        "missing is pd.NA; reporting it as a float NaN would mis-state which "
        "lane was measured"
    )
    assert values[1]["kind"] == "float64"

    payload["operation"] = "groupby_var"
    assert oracle.dispatch(pd, payload)["expected_series"]["values"][0] == {
        "kind": "null",
        "value": "null",
    }

    # FLOAT64 payload -> numpy lane -> genuine float NaN. This is the original
    # assertion, unchanged: the oracle must NOT collapse a real NaN to a generic
    # null just because FrankenPandas cannot tell them apart.
    float_payload = copy.deepcopy(payload)
    float_payload["operation"] = "groupby_std"
    float_payload["right"]["values"] = [
        {"kind": "float64", "value": 10.0},
        {"kind": "float64", "value": 20.0},
        {"kind": "null", "value": "na_n"},
        {"kind": "float64", "value": 40.0},
        {"kind": "float64", "value": 50.0},
    ]
    float_values = oracle.dispatch(pd, float_payload)["expected_series"]["values"]
    assert float_values[0] == {"kind": "null", "value": "na_n"}, (
        "an undefined group std on a float64 lane is a float NaN; rewriting it "
        "to a generic null adapts the oracle to FrankenPandas instead of "
        "measuring pandas"
    )

    float_payload["operation"] = "groupby_var"
    assert oracle.dispatch(pd, float_payload)["expected_series"]["values"][0] == {
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


def test_dtype_check_ops_report_pandas_own_spelling(oracle, pd):
    """`column_dtype_check` / `series_dtype_check` report `str(series.dtype)`.

    br-frankenpandas-62d1s. These two operations had NO oracle implementation,
    so seven corpus fixtures asserted a dtype that pandas was never asked about.
    They could not be implemented while the harness compared
    `format!("{:?}", dtype)` — Rust's `Debug` derive — because that yields
    FrankenPandas internals (`Float64`, `Utf8`, `Categorical`) with no pandas
    counterpart. The harness now compares `DType::name()`, documented "Matches
    numpy dtype.name property", so the two vocabularies coincide and this op
    needs NO translation table. That distinction is the point: a translation
    would have been the oracle-adapted-to-FP masking pattern.
    """
    def dtype_of(payload):
        return oracle.dispatch(pd, payload)["expected_dtype"]

    def series_payload(values, **extra):
        return {
            "operation": "column_dtype_check",
            "left": {
                "name": "probe",
                "index": [{"kind": "int64", "value": i} for i in range(len(values))],
                "values": values,
            },
            **extra,
        }

    # Plain numpy lanes.
    assert dtype_of(series_payload([{"kind": "float64", "value": 1.5}])) == "float64"
    assert dtype_of(series_payload([{"kind": "int64", "value": 1}])) == "int64"
    assert dtype_of(series_payload([{"kind": "utf8", "value": "x"}])) == "object"

    # int + null is the NULLABLE extension, per series_dtype_for_payload_values.
    # This is the arm that makes DISC-011 visible rather than masked: FP builds a
    # plain Int64 there and answers 'int64'.
    assert (
        dtype_of(
            series_payload(
                [{"kind": "int64", "value": 1}, {"kind": "null", "value": "null"}]
            )
        )
        == "Int64"
    )

    # CATEGORICAL: values are codes, so a plain Series would report int64 and the
    # op would answer about the wrong object entirely.
    categorical = series_payload(
        [{"kind": "int64", "value": 0}, {"kind": "int64", "value": 1}],
        categorical_categories=[
            {"kind": "utf8", "value": "low"},
            {"kind": "utf8", "value": "high"},
        ],
        categorical_ordered=True,
    )
    categorical["operation"] = "series_dtype_check"
    assert dtype_of(categorical) == "category"

    # SPARSE carries its subtype and fill in the dtype string.
    sparse = series_payload(
        [{"kind": "int64", "value": 1}, {"kind": "int64", "value": 0}],
        constructor_dtype="int64",
        fill_value={"kind": "int64", "value": 0},
    )
    sparse["operation"] = "series_dtype_check"
    assert dtype_of(sparse) == "Sparse[int64, 0]"


# ---------------------------------------------------------------------------
# br-frankenpandas-f9xlz — the adapter must not answer questions that belong to
# pandas.
#
# Only `error_origin == pandas` unlocks an error-agreement attestation
# (scripts/regenerate_fixtures.py). A refusal the ADAPTER raised from its own
# argument validation is true but vacuous — pandas was never invoked — so those
# fixtures are permanently unstampable. Each test below names the corpus fixture
# that was stuck in that state, and pins the message PANDAS actually produces.
#
# MEASURED, live pandas 2.2.3, by putting each case to pandas directly rather
# than through the oracle.
# ---------------------------------------------------------------------------


def test_dataframe_shift_out_of_range_axis_is_a_pandas_error(oracle, pd):
    """fp_p2d_144_dataframe_shift_invalid_axis_strict was origin=oracle_adapter."""
    payload = {
        "operation": "dataframe_shift",
        "frame": _frame_payload({"a": [1]}),
        "shift_axis": 2,
        "shift_periods": 1,
    }

    with pytest.raises(oracle.OracleError) as exc_info:
        oracle.dispatch(pd, payload)

    assert str(exc_info.value) == (
        "dataframe_shift failed: No axis named 2 for object type DataFrame"
    )
    assert oracle.oracle_error_origin(exc_info.value) == oracle.ERROR_ORIGIN_PANDAS


def test_dataframe_pct_change_out_of_range_axis_is_a_pandas_error(oracle, pd):
    """The unexercised twin of dataframe_shift's pre-refusal, fixed with it."""
    payload = {
        "operation": "dataframe_pct_change",
        "frame": _frame_payload({"a": [1, 2]}),
        "pct_change_axis": 2,
    }

    with pytest.raises(oracle.OracleError) as exc_info:
        oracle.dispatch(pd, payload)

    assert str(exc_info.value) == (
        "dataframe_pct_change failed: No axis named 2 for object type DataFrame"
    )
    assert oracle.oracle_error_origin(exc_info.value) == oracle.ERROR_ORIGIN_PANDAS


def test_dataframe_shift_valid_axis_still_computes(oracle, pd):
    """Control: removing the guard must not change the answer on a good axis."""
    payload = {
        "operation": "dataframe_shift",
        "frame": _frame_payload({"a": [1, 2, 3]}),
        "shift_axis": 0,
        "shift_periods": 1,
    }
    response = oracle.dispatch(pd, payload)
    values = [v["value"] for v in response["expected_frame"]["columns"]["a"]]
    assert values[1:] == [1.0, 2.0]


def test_dataframe_merge_unknown_validate_is_a_pandas_error(oracle, pd):
    """fp_p2d_035_dataframe_merge_validate_invalid_value_error_strict.

    pandas names every accepted spelling in its own message; the adapter's
    whitelist was pre-empting it.
    """
    payload = {
        "operation": "dataframe_merge",
        "frame": _frame_payload({"id": [1], "left_v": [10]}),
        "frame_right": _frame_payload({"id": [1], "right_v": [100]}),
        "join_type": "inner",
        "merge_on": "id",
        "merge_validate": "diagonal",
    }

    with pytest.raises(oracle.OracleError) as exc_info:
        oracle.dispatch(pd, payload)

    message = str(exc_info.value)
    assert message.startswith('dataframe_merge failed: "diagonal" is not a valid argument')
    assert oracle.oracle_error_origin(exc_info.value) == oracle.ERROR_ORIGIN_PANDAS


def test_dataframe_merge_recognized_validate_aliases_still_reach_pandas(oracle, pd):
    """Control: '1:1' must keep working, and must still ENFORCE one-to-one.

    A naive "just pass everything through" that also dropped the alias mapping
    would fail here, and so would one that silently stopped forwarding
    `validate` at all — the duplicate left key below has to be caught.
    """
    ok = {
        "operation": "dataframe_merge",
        "frame": _frame_payload({"id": [1], "left_v": [10]}),
        "frame_right": _frame_payload({"id": [1], "right_v": [100]}),
        "join_type": "inner",
        "merge_on": "id",
        "merge_validate": "1:1",
    }
    assert "expected_frame" in oracle.dispatch(pd, ok)

    violating = copy.deepcopy(ok)
    violating["frame"] = _frame_payload({"id": [1, 1], "left_v": [10, 11]})
    with pytest.raises(oracle.OracleError) as exc_info:
        oracle.dispatch(pd, violating)
    assert "one-to-one" in str(exc_info.value)
    assert oracle.oracle_error_origin(exc_info.value) == oracle.ERROR_ORIGIN_PANDAS


def test_dataframe_merge_cross_with_keys_is_a_pandas_error(oracle, pd):
    """fp_p2d_039_dataframe_merge_cross_rejects_keys_strict."""
    payload = {
        "operation": "dataframe_merge",
        "frame": _frame_payload({"id": [1]}),
        "frame_right": _frame_payload({"id": [2]}),
        "join_type": "cross",
        "merge_on": "id",
    }

    with pytest.raises(oracle.OracleError) as exc_info:
        oracle.dispatch(pd, payload)

    assert str(exc_info.value) == (
        "dataframe_merge failed: Can not pass on, right_on, left_on or set "
        "right_index=True or left_index=True"
    )
    assert oracle.oracle_error_origin(exc_info.value) == oracle.ERROR_ORIGIN_PANDAS


def test_dataframe_merge_cross_with_left_index_only_is_a_pandas_error(oracle, pd):
    """fp_p2d_039_dataframe_merge_cross_rejects_index_flags_hardened.

    Note this fixture sets left_index ALONE. pandas refuses on either flag, so
    the adapter must forward exactly the flag it was given rather than
    normalizing both on.
    """
    payload = {
        "operation": "dataframe_merge",
        "frame": _frame_payload({"left_v": [10]}),
        "frame_right": _frame_payload({"right_v": [20]}),
        "join_type": "cross",
        "left_index": True,
    }

    with pytest.raises(oracle.OracleError) as exc_info:
        oracle.dispatch(pd, payload)

    assert str(exc_info.value) == (
        "dataframe_merge failed: Can not pass on, right_on, left_on or set "
        "right_index=True or left_index=True"
    )
    assert oracle.oracle_error_origin(exc_info.value) == oracle.ERROR_ORIGIN_PANDAS


def test_dataframe_merge_clean_cross_still_produces_the_cartesian_product(oracle, pd):
    """Control: a cross join with NO conflicting selector must still succeed."""
    payload = {
        "operation": "dataframe_merge",
        "frame": _frame_payload({"left_v": [10, 11]}),
        "frame_right": _frame_payload({"right_v": [20, 21]}),
        "join_type": "cross",
    }
    response = oracle.dispatch(pd, payload)
    assert [v["value"] for v in response["expected_frame"]["columns"]["left_v"]] == [
        10,
        10,
        11,
        11,
    ]
    assert [v["value"] for v in response["expected_frame"]["columns"]["right_v"]] == [
        20,
        21,
        20,
        21,
    ]


def test_dataframe_merge_overlapping_columns_without_suffixes_is_a_pandas_error(
    oracle, pd
):
    """fp_p2d_036_dataframe_merge_suffixes_missing_error_strict.

    pandas DID raise here all along. The merge call sat outside every adapter
    try-block, so the exception escaped to main()'s catch-all and was labelled
    `unexpected` — which blocks attestation just as `oracle_adapter` does.
    """
    payload = {
        "operation": "dataframe_merge",
        "frame": _frame_payload({"id": [1], "val": [10]}),
        "frame_right": _frame_payload({"id": [1], "val": [100]}),
        "join_type": "inner",
        "merge_on": "id",
        "merge_suffixes": ["", ""],
    }

    with pytest.raises(oracle.OracleError) as exc_info:
        oracle.dispatch(pd, payload)

    assert "columns overlap but no suffix specified" in str(exc_info.value)
    assert oracle.oracle_error_origin(exc_info.value) == oracle.ERROR_ORIGIN_PANDAS


def test_dataframe_merge_suffixes_causing_duplicates_is_a_pandas_error(oracle, pd):
    """fp_p2d_036_dataframe_merge_suffixes_duplicate_output_error_hardened."""
    payload = {
        "operation": "dataframe_merge",
        "frame": _frame_payload({"id": [1], "val": [10], "val_L": [77]}),
        "frame_right": _frame_payload({"id": [1], "val": [100]}),
        "join_type": "inner",
        "merge_on": "id",
        "merge_suffixes": ["_L", "_R"],
    }

    with pytest.raises(oracle.OracleError) as exc_info:
        oracle.dispatch(pd, payload)

    assert "duplicate columns" in str(exc_info.value)
    assert oracle.oracle_error_origin(exc_info.value) == oracle.ERROR_ORIGIN_PANDAS


def test_a_malformed_merge_payload_is_STILL_the_adapter(oracle, pd):
    """THE NEGATIVE CASE, and the reason this change is not just a loosening.

    A blanket "wrap everything and call it pandas" would turn genuinely
    malformed fixtures into forged attestations. A payload that does not
    describe a runnable case must still classify as `oracle_adapter`, because
    pandas still never sees it.
    """
    payload = {
        "operation": "dataframe_merge",
        "frame": _frame_payload({"id": [1]}),
        "join_type": "inner",
        "merge_on": "id",
    }

    with pytest.raises(oracle.OracleError) as exc_info:
        oracle.dispatch(pd, payload)

    assert "requires frame and frame_right payloads" in str(exc_info.value)
    assert oracle.oracle_error_origin(exc_info.value) == oracle.ERROR_ORIGIN_ADAPTER
    assert oracle.oracle_error_origin(exc_info.value) != oracle.ERROR_ORIGIN_PANDAS


def test_a_non_boolean_left_index_on_a_cross_merge_is_STILL_the_adapter(oracle, pd):
    """The type checks inside the cross helper are malformed-fixture checks.

    They must survive the conversion from "raise the refusal" to "return the
    selectors": a string where a boolean belongs is not a question about pandas.
    """
    payload = {
        "operation": "dataframe_merge",
        "frame": _frame_payload({"left_v": [10]}),
        "frame_right": _frame_payload({"right_v": [20]}),
        "join_type": "cross",
        "left_index": "yes",
    }

    with pytest.raises(oracle.OracleError) as exc_info:
        oracle.dispatch(pd, payload)

    assert "left_index must be a boolean when provided" in str(exc_info.value)
    assert oracle.oracle_error_origin(exc_info.value) == oracle.ERROR_ORIGIN_ADAPTER


# ---------------------------------------------------------------------------
# br-frankenpandas-6k29f — a hand-rolled `pd.Series(values, index=index)`
# silently retypes the input, and every downstream answer inherits it.
#
# `fixture_series_from_payload` applies the payload's DECLARED dtype
# (series_dtype_for_payload_values), so an int column carrying a null builds as
# nullable Int64. The 46 hand-rolled sites skip that and let pandas infer
# numpy float64 from a bare value list.
#
# THE SITES CHANGED HERE WERE CHOSEN BY MEASUREMENT, not by grep. Of the 46
# sites, only 9 are exercised by a corpus fixture whose declared dtype differs
# from bare inference at all, and of those only series_sort_values and
# series_tail actually change their ANSWER — the other seven are pinned below
# as controls precisely because they must NOT be swept in with them.
# ---------------------------------------------------------------------------


def _int_with_null_payload(values, index):
    """A payload whose declared dtype (Int64) differs from bare inference."""
    return {
        "index": [{"kind": "utf8", "value": i} for i in index],
        "values": [
            {"kind": "null", "value": "null"} if v is None
            else {"kind": "int64", "value": v}
            for v in values
        ],
        "name": "s",
    }


def test_series_tail_keeps_the_declared_int64_dtype_6k29f(oracle, pd):
    """fp_p2d_044_series_tail_negative_preserves_nulls_hardened.

    tail() is a pure row selection, so the input dtype IS the output dtype. The
    hand-rolled builder returned float64 3.0 and an `na_n` marker where the
    corpus declares Int64 3 and a `null`.
    """
    payload = {
        "operation": "series_tail",
        "left": _int_with_null_payload([1, None, 3, None], ["r1", "r2", "r3", "r4"]),
        "tail_n": 3,
    }
    response = oracle.dispatch(pd, payload)
    values = response["expected_series"]["values"]

    assert [v["kind"] for v in values] == ["null", "int64", "null"], (
        "an int64+null payload must stay int64; float64 here means the handler "
        "is still hand-rolling the Series"
    )
    assert values[1]["value"] == 3
    assert values[0]["value"] == "null", "declared Int64 carries pd.NA, not NaN"


def test_series_sort_values_keeps_the_declared_int64_dtype_6k29f(oracle, pd):
    """fp_p2d_043_series_sort_values_numeric_descending_na_last_hardened."""
    payload = {
        "operation": "series_sort_values",
        "left": _int_with_null_payload([1, 3, None, 2], ["r1", "r2", "r3", "r4"]),
        "sort_ascending": False,
    }
    response = oracle.dispatch(pd, payload)
    values = response["expected_series"]["values"]
    index = [i["value"] for i in response["expected_series"]["index"]]

    assert [v["kind"] for v in values] == ["int64", "int64", "int64", "null"]
    assert [v["value"] for v in values[:3]] == [3, 2, 1]
    # na_position="last" must survive the dtype change — a nullable Int64 sorts
    # pd.NA differently from how float64 sorts NaN if the argument is dropped.
    assert index == ["r2", "r4", "r1", "r3"]


def test_series_sort_values_all_valid_int_payload_is_unaffected_6k29f(oracle, pd):
    """Control: no null means no nullable-extension question, so nothing moves.

    `series_dtype_for_payload_values` returns plain int64 here, so this row
    reads identically before and after the change. If it did move, the shared
    builder would be imposing a dtype rather than applying the declared one.
    """
    payload = {
        "operation": "series_sort_values",
        "left": _int_with_null_payload([3, 1, 2], ["r1", "r2", "r3"]),
        "sort_ascending": True,
    }
    values = oracle.dispatch(pd, payload)["expected_series"]["values"]
    assert [v["kind"] for v in values] == ["int64", "int64", "int64"]
    assert [v["value"] for v in values] == [1, 2, 3]


@pytest.mark.parametrize(
    "operation",
    ["series_isna", "series_notna", "series_isnull", "series_notnull"],
)
def test_null_predicate_ops_are_NOT_part_of_this_defect_6k29f(oracle, pd, operation):
    """THE NEGATIVE CONTROL: these hand-roll too, and must be left alone.

    The bead warns that "the 49 remaining sites are NOT all defects". Measured
    over the corpus: of the 9 hand-rolled ops a divergent fixture reaches, seven
    produce a byte-identical answer under either builder, because their output
    cannot observe the input dtype. isna/notna emit booleans about NULL-NESS,
    which is the same set of positions whether the column is float64-with-NaN or
    Int64-with-pd.NA.

    Pinned so that a future bulk sweep of all 46 sites has to argue with a test
    rather than with a comment.
    """
    payload = {
        "operation": operation,
        "left": _int_with_null_payload([1, None, 3], ["r1", "r2", "r3"]),
    }
    values = oracle.dispatch(pd, payload)["expected_series"]["values"]
    assert [v["kind"] for v in values] == ["bool", "bool", "bool"]
    truthy = [v["value"] for v in values]
    if operation in ("series_isna", "series_isnull"):
        assert truthy == [False, True, False]
    else:
        assert truthy == [True, False, True]


def test_series_count_is_NOT_part_of_this_defect_6k29f(oracle, pd):
    """The bead names count explicitly as a non-defect; measured and pinned.

    count() returns a scalar number of non-missing entries. float64-with-NaN and
    Int64-with-pd.NA have the same missing positions, so the count is the same.
    """
    payload = {
        "operation": "series_count",
        "left": _int_with_null_payload([1, None, 3, None], ["r1", "r2", "r3", "r4"]),
    }
    response = oracle.dispatch(pd, payload)
    assert response["expected_scalar"]["value"] == 2


def test_timedelta_total_seconds_on_a_numeric_series_is_a_pandas_error_f9xlz(oracle, pd):
    """fp_p2d_022_series_timedelta_total_seconds_numeric_passthrough_strict.

    The adapter refuses a numeric series here on purpose (br-frankenpandas-7btvv:
    to_timedelta would otherwise read the numbers as NANOSECONDS and manufacture
    an answer pandas never gives). The refusal is right; it was the ORIGIN that
    was wrong, because the message was a hand-copied transcription of pandas'
    and pandas was never actually asked.

    MEASURED, live pandas 2.2.3, on this fixture's own payload:
        pd.Series([60, 3661.5, 86400]).dt.total_seconds()
          -> AttributeError: Can only use .dt accessor with datetimelike values
    """
    payload = {
        "operation": "series_timedelta_total_seconds",
        "left": {
            "index": [{"kind": "int64", "value": i} for i in range(3)],
            "values": [
                {"kind": "int64", "value": 60},
                {"kind": "float64", "value": 3661.5},
                {"kind": "int64", "value": 86400},
            ],
            "name": "s",
        },
    }

    with pytest.raises(oracle.OracleError) as exc_info:
        oracle.dispatch(pd, payload)

    assert "Can only use .dt accessor with datetimelike values" in str(exc_info.value)
    assert oracle.oracle_error_origin(exc_info.value) == oracle.ERROR_ORIGIN_PANDAS


def test_timedelta_total_seconds_still_computes_for_real_timedeltas_f9xlz(oracle, pd):
    """Control: the legitimate path must be untouched.

    Gate 1 fires on the same condition as before; only the way it refuses moved.
    A timedelta-string column must still round-trip through to_timedelta.
    """
    payload = {
        "operation": "series_timedelta_total_seconds",
        "left": {
            "index": [{"kind": "int64", "value": i} for i in range(3)],
            "values": [
                {"kind": "utf8", "value": "1 days 00:00:00"},
                {"kind": "null", "value": "null"},
                {"kind": "utf8", "value": "0 days 01:30:00"},
            ],
            "name": "s",
        },
    }
    values = oracle.dispatch(pd, payload)["expected_series"]["values"]
    assert values[0]["value"] == 86400.0
    assert values[1]["kind"] == "null"
    assert values[2]["value"] == 5400.0


def test_startswith_endswith_accept_an_empty_and_whitespace_pattern(oracle, pd):
    """br-frankenpandas-9rop8: the oracle REFUSED input live pandas accepts.

    `required_string_payload` rejects `""` and `.strip()`s what it returns. That
    is right for a regex and wrong for a literal prefix/suffix, and it went wrong
    two ways at once:

      * `""` is a valid prefix — every string starts with it. MEASURED, live
        pandas 2.2.3 on `['abc','bcd',None]`, `.str.startswith('')` is
        `[True, True, None]`. The oracle refused to ask, so
        `fp_p2d_174`/`fp_p2d_175_series_str_(starts|ends)with_empty_pattern_strict`
        pinned answers nothing had verified — while the fixtures themselves
        already expected all-True, i.e. FrankenPandas was right the whole time.
      * stripping MUTATES the pattern. `startswith(" ")` asks about a leading
        space; stripped it becomes `startswith("")`, a different question — and
        it did not even get that far, because the stripped value is empty and was
        then refused as such.
    """
    def payload(op, pattern):
        return {
            "operation": op,
            "left": {
                "name": "text",
                "index": [
                    {"kind": "int64", "value": 0},
                    {"kind": "int64", "value": 1},
                ],
                "values": [
                    {"kind": "utf8", "value": "abc"},
                    {"kind": "utf8", "value": " lead"},
                ],
            },
            "regex_pattern": pattern,
        }

    def values_of(op, pattern):
        out = oracle.dispatch(pd, payload(op, pattern))
        return [v["value"] for v in out["expected_series"]["values"]]

    # An empty prefix/suffix matches everything, and must no longer raise.
    assert values_of("series_str_startswith", "") == [True, True]
    assert values_of("series_str_endswith", "") == [True, True]

    # Whitespace is significant and must survive verbatim: " lead" starts with
    # " " and "abc" does not. Stripping would have collapsed this to the
    # empty-pattern case above and answered [True, True].
    assert values_of("series_str_startswith", " ") == [False, True]

    # A non-string is still refused — the guard was narrowed, not removed.
    with pytest.raises(Exception):
        oracle.dispatch(pd, payload("series_str_startswith", 5))


def test_dataframe_compare_reports_pandas_two_level_column_axis(oracle, pd):
    """`DataFrame.compare` had NO oracle handler (br-frankenpandas-nvnvr).

    fp_p2d_418 therefore asserted a result pandas was never asked for — the same
    unverifiable state br-frankenpandas-62d1s found for the dtype-check ops.

    MEASURED, live pandas 2.2.3, left {'a':[1,2],'b':[3,4]} vs right
    {'a':[1,9],'b':[3,4]} with result_names=('left','right'):

        index   [1]                       only the differing row survives
        columns [('a','left'), ('a','right')]   TWO-LEVEL, and 'b' is dropped
        values  2.0 / 9.0                 float64 — compare promotes, because
                                          unmatched cells become NaN

    The handler adds NO flattening of its own: `dataframe_to_json` already maps a
    two-level axis to `'a_left'`/`'a_right'` with `'_'.join` and carries the
    tuples in `column_multiindex` (br-frankenpandas-nv5ct). Inventing a bespoke
    flattening here would be the oracle-adapted-to-FP masking pattern.
    """
    def frame(a_values):
        return {
            "index": [{"kind": "int64", "value": 0}, {"kind": "int64", "value": 1}],
            "columns": {
                "a": [{"kind": "int64", "value": v} for v in a_values],
                "b": [{"kind": "int64", "value": 3}, {"kind": "int64", "value": 4}],
            },
            "column_order": ["a", "b"],
        }

    out = oracle.dispatch(
        pd,
        {
            "operation": "dataframe_compare",
            "frame": frame([1, 2]),
            "frame_right": frame([1, 9]),
            "compare_result_names": ["left", "right"],
        },
    )["expected_frame"]

    # Only the differing row, and only the differing column, survive.
    assert [v["value"] for v in out["index"]] == [1]
    assert out["column_order"] == ["a_left", "a_right"]
    assert "b_left" not in out["columns"]

    # pandas PROMOTES to float64 here; the values are 2.0 / 9.0, not 2 / 9.
    assert [c["kind"] for c in out["columns"]["a_left"]] == ["float64"]
    assert out["columns"]["a_left"][0]["value"] == 2.0
    assert out["columns"]["a_right"][0]["value"] == 9.0

    # The flattened keys are a VIEW; the tuples travel losslessly beside them.
    assert out.get("column_multiindex") is not None

    # result_names is validated, not silently ignored.
    with pytest.raises(Exception):
        oracle.dispatch(
            pd,
            {
                "operation": "dataframe_compare",
                "frame": frame([1, 2]),
                "frame_right": frame([1, 9]),
                "compare_result_names": ["only-one"],
            },
        )


def test_series_to_arrow_round_trip_preserves_dtype_index_and_nulls(oracle, pd):
    """`series_to_arrow_round_trip` had NO oracle handler (br-frankenpandas-nvnvr).

    The PATH is the substance of this handler, not boilerplate. MEASURED, live
    pandas 2.2.3 + pyarrow 24.0.0 on Series([10, <NA>, 30], index=r0..r2,
    dtype='Int64'):

        pa.Array.from_pandas(s).to_pandas()   -> [10.0, nan, 30.0]  float64
        pa.Table.from_pandas(s.to_frame())    -> [10, <NA>, 30]     Int64, index kept
            .to_pandas()[name]

    A bare Arrow ARRAY has no index and no pandas metadata, so it cannot express
    a Series round trip: it drops the index and demotes nullable Int64 to
    float64. Picking the Array path would have quietly redefined the operation
    into something that always loses information — and the fixture would then
    have "agreed" with a weaker claim.
    """
    out = oracle.dispatch(
        pd,
        {
            "operation": "series_to_arrow_round_trip",
            "left": {
                "name": "vals",
                "index": [
                    {"kind": "utf8", "value": "r0"},
                    {"kind": "utf8", "value": "r1"},
                    {"kind": "utf8", "value": "r2"},
                ],
                "values": [
                    {"kind": "int64", "value": 10},
                    {"kind": "null", "value": "null"},
                    {"kind": "int64", "value": 30},
                ],
            },
        },
    )["expected_series"]

    # The index survives — an Arrow array alone could not have carried it.
    assert [v["value"] for v in out["index"]] == ["r0", "r1", "r2"]

    # Values keep their INTEGER kind rather than being demoted to float64,
    # and the gap stays a null rather than becoming NaN.
    kinds = [v["kind"] for v in out["values"]]
    assert kinds == ["int64", "null", "int64"], kinds
    assert out["values"][0]["value"] == 10
    assert out["values"][2]["value"] == 30
