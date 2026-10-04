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
