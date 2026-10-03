"""Additional actual-pandas controls for the projected shared-key branch.

Requires the actual extension and pandas, without skips.
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


@pytest.mark.parametrize("how", ("inner", "left", "right", "outer"))
def test_mirrored_shared_float_key_preserves_projected_side(how, indicator):
    def run(module):
        left = module.DataFrame({
            "k": np.array([1.0, 1.00000003], dtype="float64"), "l": [10, 20],
        })
        right = module.DataFrame({
            "k": np.array([1.0, 2.0], dtype="float32"), "r": [30, 40],
        })
        return module.merge(left, right, on="k", how=how, indicator=indicator)

    assert_positional_equal(run(fp), run(pd))


@pytest.mark.parametrize("left_dtype,right_dtype", (
    ("float32", "float64"), ("float64", "float32"),
))
def test_all_matched_right_key_preserves_left_source_width(left_dtype, right_dtype, indicator):
    def run(module):
        left = module.DataFrame({
            "k": np.array([1.0, 2.0], dtype=left_dtype), "l": [10, 20],
        })
        right = module.DataFrame({
            "k": np.array([1.0, 2.0], dtype=right_dtype), "r": [30, 40],
        })
        return module.merge(left, right, on="k", how="right", indicator=indicator)

    assert_positional_equal(run(fp), run(pd))
