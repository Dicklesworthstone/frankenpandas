"""Real-oracle controls for positional merge-width restoration.

Tests require the actual extension and pandas; no skips.
All output columns are compared by position, including repeated display names.
"""

import math

import frankenpandas as fp
import numpy as np
import pandas as pd
import pytest


@pytest.fixture(params=(False, True), ids=("default_fast_paths", "indicator_generic_path"))
def indicator(request):
    return request.param


def cells(column):
    return [
        None if value is None or value is pd.NA or value is fp.NA
        or isinstance(value, (float, np.floating)) and math.isnan(value)
        else value
        for value in column.tolist()
    ]


def assert_positional_equal(actual, expected):
    assert actual.shape == expected.shape
    assert list(actual.columns) == list(expected.columns)
    assert [str(dtype) for dtype in actual.dtypes] == [
        str(dtype) for dtype in expected.dtypes
    ]
    for position in range(expected.shape[1]):
        assert cells(actual.iloc[:, position]) == cells(expected.iloc[:, position])


def assert_temporal_merge_equal(actual, expected):
    """Compare the entire frame, including row labels, names and key widths."""
    assert_positional_equal(actual, expected)
    assert type(actual.index).__name__ == type(expected.index).__name__
    assert str(actual.index.dtype) == str(expected.index.dtype)
    assert actual.index.tolist() == expected.index.tolist()
    assert list(actual.index.names) == list(expected.index.names)
    assert list(actual.columns.names) == list(expected.columns.names)


@pytest.mark.parametrize("how", ("outer", "inner", "left", "right"))
@pytest.mark.parametrize("fill_method", (None, "ffill"))
@pytest.mark.parametrize("keys", ("shared", "different", "inferred", "composite"))
def test_ordered_merge_independent_keys_direction_and_source_fill(how, fill_method, keys):
    def run(module):
        left = module.DataFrame({
            "t": [3, 1, 3, 6],
            "g": ["a", "a", "b", "b"],
            "v": np.array([30.5, 10.5, np.nan, 60.5], dtype="float32"),
        })
        right = module.DataFrame({
            "t": [2, 3, 5], "g": ["a", "b", "b"], "v": [20.0, np.nan, 50.0],
        })
        if keys == "shared":
            kwargs = {"on": "t"}
        elif keys == "different":
            right = right.rename(columns={"t": "rt"})
            kwargs = {"left_on": "t", "right_on": "rt"}
        elif keys == "composite":
            right = right.rename(columns={"t": "rt"})
            kwargs = {"left_on": ["t", "g"], "right_on": ["rt", "g"]}
        else:
            right = right.rename(columns={"g": "rg", "v": "rv"})
            kwargs = {}
        return module.merge_ordered(
            left, right, how=how, fill_method=fill_method,
            suffixes=("_left", "_right"), **kwargs,
        )

    assert_temporal_merge_equal(run(fp), run(pd))


@pytest.mark.parametrize("side", ("left", "right"))
@pytest.mark.parametrize("how", ("outer", "inner", "left", "right"))
@pytest.mark.parametrize("fill_method", (None, "ffill"))
@pytest.mark.parametrize("other_grouped", (False, True))
def test_ordered_merge_groups_are_independent(side, how, fill_method, other_grouped):
    def run(module):
        grouped = module.DataFrame({
            "g": ["b", "a", "b", "a", "missing"],
            "t": [3, 1, 6, 4, 2], "v": [30.0, 10.0, np.nan, 40.0, 20.0],
        })
        other = module.DataFrame({"t": [2, 3, 5], "v": [20.0, np.nan, 50.0]})
        if other_grouped:
            other["g"] = ["a", "b", "a"]
        left, right = (grouped, other) if side == "left" else (other, grouped)
        return module.merge_ordered(
            left, right, on="t", how=how, fill_method=fill_method,
            **{f"{side}_by": "g"},
        )

    assert_temporal_merge_equal(run(fp), run(pd))


@pytest.mark.parametrize("suffixes", (("_x", "_y"), (None, "_right"), ("_left", None), ("", "_right")))
def test_ordered_merge_suffixes_and_key_aliases(suffixes):
    def run(module):
        # Both sides hold both key names, so dropping right_on silently
        # produces a plausible but wrong join instead of just raising.
        left = module.DataFrame({"lk": [1, 3], "rk": [100, 200], "v": [10, 30]})
        right = module.DataFrame({"lk": [90, 91], "rk": [2, 3], "v": [20, 30]})
        return module.merge_ordered(left, right, left_on="lk", right_on="rk", suffixes=suffixes)

    assert_temporal_merge_equal(run(fp), run(pd))


def test_ordered_merge_typed_keys_and_named_series():
    for make in (
        lambda m: m.merge_ordered(m.DataFrame({0: [1, 3], 1: [10, 30]}), m.DataFrame({0: [2, 3], 2: [20, 30]}), on=0),
        lambda m: m.merge_ordered(m.Series([1, 3], name="t"), m.Series([2, 3], name="t"), on="t"),
        lambda m: m.merge_ordered(m.DataFrame({"t": m.to_datetime(["2024-01-01", "2024-01-03"]), "v": [1, 3]}), m.DataFrame({"t": m.to_datetime(["2024-01-02"]), "w": [2]}), on="t", fill_method="ffill"),
    ):
        assert_temporal_merge_equal(make(fp), make(pd))


@pytest.mark.parametrize("kwargs", (
    {"left_on": "t"}, {"right_on": "t"},
    {"on": "t", "left_on": "t", "right_on": "t"},
    {"on": "t", "left_by": "g", "right_by": "g"},
    {"left_on": ["t", "g"], "right_on": "t"},
    {"on": "t", "suffixes": (None, None)},
    {"on": "missing"}, {"on": []},
    {"on": "t", "left_by": "missing"},
    {"on": "t", "right_by": "missing"},
    {"left_on": "t", "right_on": "missing"},
))
def test_ordered_merge_invalid_arguments_do_not_silently_change_keys(kwargs):
    def run(module):
        frame = module.DataFrame({"t": [1, 2], "g": ["a", "b"], "v": [1, 2]})
        return module.merge_ordered(frame, frame, **kwargs)

    with pytest.raises(Exception) as oracle:
        run(pd)
    with pytest.raises(Exception) as actual:
        run(fp)
    assert type(actual.value).__name__ == type(oracle.value).__name__


@pytest.mark.parametrize("direction", ("backward", "forward", "nearest"))
@pytest.mark.parametrize("allow_exact_matches", (False, True))
@pytest.mark.parametrize("grouping", (None, "shared", "different", "composite"))
@pytest.mark.parametrize("tolerance", (None, 1))
def test_asof_independent_ordering_and_grouping_keys(direction, allow_exact_matches, grouping, tolerance):
    def run(module):
        left = module.DataFrame({
            "lt": [1, 2, 3, 4, 6], "lg": ["a", "b", "a", "b", "a"],
            "h": [1, 1, 2, 1, 2], "v": np.array([10, 20, 30, 40, 60], dtype="int16"),
        })
        right = module.DataFrame({
            "rt": [2, 3, 3, 5], "rg": ["a", "b", "a", "a"],
            "h": [1, 1, 2, 2], "v": np.array([20, 30, 31, 50], dtype="float32"),
        })
        kwargs = {}
        if grouping == "shared":
            left = left.rename(columns={"lg": "g"})
            right = right.rename(columns={"rg": "g"})
            kwargs = {"by": "g"}
        elif grouping == "different":
            kwargs = {"left_by": "lg", "right_by": "rg"}
        elif grouping == "composite":
            kwargs = {"left_by": ["lg", "h"], "right_by": ["rg", "h"]}
        return module.merge_asof(
            left, right, left_on="lt", right_on="rt", direction=direction,
            allow_exact_matches=allow_exact_matches, tolerance=tolerance,
            suffixes=("_left", "_right"), **kwargs,
        )

    assert_temporal_merge_equal(run(fp), run(pd))


@pytest.mark.parametrize("mode", ("left_index", "right_index", "both_indexes"))
@pytest.mark.parametrize("direction", ("backward", "forward", "nearest"))
@pytest.mark.parametrize("overlap", (False, True))
@pytest.mark.parametrize("suffixes", (("_x", "_y"), (None, "_r"), ("_l", None)))
def test_asof_index_column_keys_preserve_rows_and_overlaps(mode, direction, overlap, suffixes):
    def run(module):
        left = module.DataFrame({"t": [1, 3, 6], "v": [10, 30, 60]}, index=module.Index(["a", "b", "c"], name="row"))
        right = module.DataFrame({"t": [2, 3, 5], "v": [20, 30, 50]})
        kwargs = {}
        if mode in ("left_index", "both_indexes"):
            left = left.set_index("t")
            kwargs["left_index"] = True
            if overlap:
                left["t"] = [90, 91, 92]
        else:
            kwargs["left_on"] = "t"
        if mode in ("right_index", "both_indexes"):
            right = right.set_index("t")
            kwargs["right_index"] = True
            if overlap:
                right["t"] = [80, 81, 82]
        else:
            kwargs["right_on"] = "t"
        return module.merge_asof(left, right, direction=direction, suffixes=suffixes, **kwargs)

    assert_temporal_merge_equal(run(fp), run(pd))


@pytest.mark.parametrize("dtype", ("int32", "int64", "float32", "float64"))
@pytest.mark.parametrize("mode", ("left_index", "right_index", "both_indexes"))
def test_asof_index_key_width_and_empty_inputs(dtype, mode):
    for empty_side in (None, "left", "right", "both"):
        def run(module):
            lv = [] if empty_side in ("left", "both") else [1, 3, 6]
            rv = [] if empty_side in ("right", "both") else [2, 3, 5]
            left = module.DataFrame({"t": np.array(lv, dtype=dtype), "l": np.arange(len(lv), dtype="int16")})
            right = module.DataFrame({"t": np.array(rv, dtype=dtype), "r": np.arange(len(rv), dtype="float32")})
            kwargs = {}
            if mode in ("left_index", "both_indexes"):
                left = left.set_index("t")
                kwargs["left_index"] = True
            else:
                kwargs["left_on"] = "t"
            if mode in ("right_index", "both_indexes"):
                right = right.set_index("t")
                kwargs["right_index"] = True
            else:
                kwargs["right_on"] = "t"
            return module.merge_asof(left, right, **kwargs)

        assert_temporal_merge_equal(run(fp), run(pd))


@pytest.mark.parametrize("dtype", ("int64", "float64", "bool", "int32", "float32"))
def test_empty_set_index_retains_declared_numeric_dtype(dtype):
    def run(module):
        frame = module.DataFrame({"t": np.array([], dtype=dtype)})
        return frame.set_index("t")

    assert_temporal_merge_equal(run(fp), run(pd))


def test_asof_mixed_index_key_restoration_preserves_large_ints():
    def run(module):
        base = 2**60
        left = module.DataFrame({"l": [1, 2, 3]}, index=module.Index([base + 1, base + 3, base + 7], name="time"))
        right = module.DataFrame({"rt": [base + 2, base + 3, base + 6], "r": [10, 20, 30]})
        return module.merge_asof(left, right, left_index=True, right_on="rt")

    actual, expected = run(fp), run(pd)
    assert_temporal_merge_equal(actual, expected)
    assert actual["rt"].tolist() == [2**60 + 1, 2**60 + 3, 2**60 + 7]


@pytest.mark.parametrize("dtype", ("int64", "float64"))
@pytest.mark.parametrize("numpy_side", ("left", "right"))
def test_asof_default_numeric_width_matches_inferred_columns(dtype, numpy_side):
    def run(module):
        left_values = [1, 3, 6] if dtype == "int64" else [1.0, 3.0, 6.0]
        right_values = [2, 3, 5] if dtype == "int64" else [2.0, 3.0, 5.0]
        if numpy_side == "left":
            left_values = np.array(left_values, dtype=dtype)
        else:
            right_values = np.array(right_values, dtype=dtype)
        left = module.DataFrame({"t": left_values, "l": [10, 30, 60]})
        right = module.DataFrame({"t": right_values, "r": [20, 30, 50]})
        return module.merge_asof(left, right, on="t")

    assert_temporal_merge_equal(run(fp), run(pd))


def test_asof_synthetic_index_key_never_overwrites_user_data():
    def run(module):
        left = module.DataFrame({"\x01asof-index": [11, 12, 13]}, index=[1, 3, 6])
        right = module.DataFrame({"t": [2, 3, 5], "\x01asof-index": [21, 22, 23]})
        return module.merge_asof(left, right, left_index=True, right_on="t")

    assert_temporal_merge_equal(run(fp), run(pd))


@pytest.mark.parametrize("kwargs", (
    {"left_on": "t"}, {"right_on": "t"},
    {"on": "t", "left_on": "t", "right_on": "t"},
    {"on": "t", "left_index": True},
    {"left_on": "t", "left_index": True, "right_on": "t"},
    {"on": "t", "left_by": "g"}, {"on": "t", "right_by": "g"},
    {"on": "t", "by": "g", "left_by": "g", "right_by": "g"},
    {"on": "t", "left_by": ["g", "v"], "right_by": "g"},
    {"on": "t", "direction": "sideways"},
    {"on": ["t", "v"]}, {"on": []}, {"on": "missing"},
    {"on": "t", "suffixes": (None, None)},
))
def test_asof_invalid_arguments_are_rejected(kwargs):
    def run(module):
        frame = module.DataFrame({"t": [1, 2], "g": ["a", "b"], "v": [1, 2]})
        return module.merge_asof(frame, frame, **kwargs)

    with pytest.raises(Exception) as oracle:
        run(pd)
    with pytest.raises(Exception) as actual:
        run(fp)
    assert type(actual.value).__name__ == type(oracle.value).__name__


@pytest.mark.parametrize("case", ("unsorted_left", "unsorted_right", "null_left", "null_right", "key_dtype", "group_dtype"))
def test_asof_invalid_data_keeps_pandas_exception_type(case):
    def run(module):
        left = module.DataFrame({"t": [1.0, 3.0, 6.0], "g": [1, 1, 2]})
        right = module.DataFrame({"t": [2.0, 3.0, 5.0], "g": [1, 1, 2]})
        if case.startswith("unsorted"):
            if case.endswith("left"):
                left = left.iloc[::-1]
            else:
                right = right.iloc[::-1]
        elif case.startswith("null"):
            if case.endswith("left"):
                left["t"] = [1.0, np.nan, 6.0]
            else:
                right["t"] = [2.0, np.nan, 5.0]
        elif case == "key_dtype":
            right["t"] = right["t"].astype("int64")
        else:
            right["g"] = right["g"].astype("float64")
        return module.merge_asof(left, right, on="t", by="g")

    with pytest.raises(Exception) as oracle:
        run(pd)
    with pytest.raises(Exception) as actual:
        run(fp)
    assert type(actual.value).__name__ == type(oracle.value).__name__


def test_asof_right_index_precedes_redundant_right_on_like_pandas():
    def run(module):
        left = module.DataFrame({"t": [1, 3, 6], "v": [10, 30, 60]}, index=["a", "b", "c"])
        right = module.DataFrame({"t": [2, 3, 5], "v": [20, 30, 50]}, index=[100, 200, 300])
        return module.merge_asof(left, right, left_on="t", right_on="t", right_index=True)

    assert_temporal_merge_equal(run(fp), run(pd))


@pytest.mark.parametrize("repeated_side", ("left", "right"))
def test_repeated_payload_names_keep_each_width(repeated_side, indicator):
    def run(module):
        repeated = module.DataFrame({
            "k": [1, 2],
            "first": np.array([3.5, 4.5], dtype="float32"),
            "second": np.array([1.00000003, 1.00000007], dtype="float64"),
        })
        repeated.columns = ["k", "v", "v"]
        plain = module.DataFrame({"k": [1, 2], "q": np.array([5, 6], dtype="int16")})
        left, right = (repeated, plain) if repeated_side == "left" else (plain, repeated)
        return module.merge(left, right, on="k", indicator=indicator)

    assert_positional_equal(run(fp), run(pd))


def test_cross_side_suffix_collision_keeps_both_payloads(indicator):
    def run(module):
        left = module.DataFrame({"k": [1], "v": np.array([3.5], dtype="float32")})
        right = module.DataFrame({"k": [1], "v": np.array([1.00000003], dtype="float64")})
        return module.merge(left, right, on="k", suffixes=("_s", "_s"), indicator="origin" if indicator else False)

    assert_positional_equal(run(fp), run(pd))


def test_partially_shared_composite_keys_and_indicator_keep_projection(indicator):
    def run(module):
        left = module.DataFrame({
            "a": np.array([1, 2], dtype="int16"),
            "lk": [10, 20],
            "v": np.array([3.5, 4.5], dtype="float32"),
        })
        right = module.DataFrame({
            "a": np.array([2, 3], dtype="int32"),
            "rk": [20, 30],
            "v": np.array([1.00000003, 1.00000007], dtype="float64"),
        })
        return module.merge(
            left, right, left_on=["a", "lk"], right_on=["a", "rk"],
            how="outer", indicator=indicator,
        )

    assert_positional_equal(run(fp), run(pd))


def test_cross_join_keeps_both_same_named_narrow_columns(indicator):
    def run(module):
        left = module.DataFrame({"v": np.array([3.5, 4.5], dtype="float32")})
        right = module.DataFrame({"v": np.array([1.00000003], dtype="float64")})
        return module.merge(left, right, how="cross", indicator=indicator)

    assert_positional_equal(run(fp), run(pd))


@pytest.mark.parametrize("how", ("inner", "left", "right", "outer"))
def test_shared_float_key_restoration_preserves_source_precision(how, indicator):
    def run(module):
        left = module.DataFrame({"k": np.array([1.0, 2.0], dtype="float32"), "l": [10, 20]})
        right = module.DataFrame({
            "k": np.array([1.0, 1.00000003], dtype="float64"), "r": [30, 40],
        })
        return module.merge(left, right, on="k", how=how, indicator=indicator)

    assert_positional_equal(run(fp), run(pd))


def test_nullable_narrow_payload_gaps_remain_missing(indicator):
    def run(module):
        left = module.DataFrame({
            "k": [1, 2], "v": module.Series([7, None], dtype="Int16"),
        })
        right = module.DataFrame({
            "k": [2, 3], "v": module.Series([8, None], dtype="Int32"),
        })
        return module.merge(left, right, on="k", how="outer", indicator=indicator)

    assert_positional_equal(run(fp), run(pd))
