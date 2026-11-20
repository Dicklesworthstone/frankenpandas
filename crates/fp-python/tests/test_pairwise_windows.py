"""Pairwise window matrices against the pinned pandas 2.2.3 oracle."""

from __future__ import annotations

import numpy as np
import pandas as pd
import pytest

import frankenpandas as fpd


def _label(actual, expected):
    if isinstance(expected, tuple):
        assert isinstance(actual, tuple)
        assert len(actual) == len(expected)
        for a, e in zip(actual, expected):
            _label(a, e)
    elif isinstance(expected, pd.Timestamp):
        assert actual.value == expected.value
        assert str(actual.tz) == str(expected.tz)
    else:
        assert actual == expected


def _axis(actual, expected):
    assert len(actual) == len(expected)
    assert actual.nlevels == expected.nlevels
    assert list(actual.names) == list(expected.names)
    for a, e in zip(actual.tolist(), expected.tolist()):
        _label(a, e)
    if expected.nlevels > 1:
        for a, e in zip(actual.levels, expected.levels):
            assert len(a) == len(e)
            for av, ev in zip(a.tolist(), e.tolist()):
                _label(av, ev)


def _result(actual, expected):
    assert isinstance(actual, fpd.DataFrame)
    assert actual.shape == expected.shape
    _axis(actual.index, expected.index)
    _axis(actual.columns, expected.columns)
    assert [str(dtype) for dtype in actual.dtypes] == [
        str(dtype) for dtype in expected.dtypes
    ]
    np.testing.assert_allclose(
        actual.to_numpy(), expected.to_numpy(), rtol=1e-12, atol=1e-12,
        equal_nan=True,
    )


def _window(frame, kind, *, adjust=True, ignore_na=False):
    if kind == "rolling":
        return frame.rolling(3, min_periods=1, center=True)
    if kind == "expanding":
        return frame.expanding(min_periods=1)
    return frame.ewm(
        alpha=0.35, adjust=adjust, ignore_na=ignore_na, min_periods=1,
    )


def _cross_frames(module, case):
    left_index = module.Index([0, 1, 2, 3, 4], name="row")
    right_index = module.Index([0, 1, 2, 3, 4], name="row")
    left_columns = module.Index(["a", "a__b"], name="left_columns")
    right_columns = module.Index(["b__c", "c", "z"], name="right_columns")
    if case == "align":
        left_index = module.Index([2, 0, 1, 4, 5], name="left_rows")
        right_index = module.Index([0, 1, 3, 4, 6], name="right_rows")
    elif case == "duplicates":
        left_index = right_index = module.Index([0, 0, 2, 3, 3], name="row")
        left_columns = module.Index(["same", "same"], name="left_columns")
        right_columns = module.Index(["same", "same", "z"], name="right_columns")
    elif case == "typed":
        left_columns = module.Index([3, 1], name="left_columns")
        right_columns = module.Index([8, 9, 7], name="right_columns")
    elif case in ("multi_columns", "unsorted_multi_columns"):
        left_columns = module.MultiIndex.from_tuples(
            [("a", 1), ("b", 2)], names=["left0", "left1"],
        )
        right_columns = module.MultiIndex.from_tuples(
            [("q", 5), ("q", 6), ("r", 7)], names=["right0", "right1"],
        )
        if case == "unsorted_multi_columns":
            left_index = right_index = module.Index([4, 1, 0, 3, 2], name="row")
            right_columns = module.MultiIndex.from_tuples(
                [("r", 7), ("q", 6), ("q", 5)], names=["right0", "right1"],
            )
    left = module.DataFrame(
        [[1.0, 8.0], [2.0, np.nan], [4.0, 3.0], [np.nan, 9.0], [7.0, 2.0]],
        index=left_index, columns=left_columns,
    )
    right = module.DataFrame(
        [[2.0, 3.0, 5.0], [6.0, 1.0, 5.0], [np.nan, 9.0, 5.0],
         [4.0, 2.0, 5.0], [8.0, 7.0, 5.0]],
        index=right_index, columns=right_columns,
    )
    return left, right


@pytest.mark.parametrize("kind", ["rolling", "expanding", "ewm"])
@pytest.mark.parametrize("operation", ["corr", "cov"])
@pytest.mark.parametrize("case", [
    "plain", "align", "duplicates", "typed", "multi_columns", "unsorted_multi_columns",
])
def test_cross_frame_pairwise_windows(kind, operation, case):
    left, right = _cross_frames(fpd, case)
    oracle_left, oracle_right = _cross_frames(pd, case)
    actual = getattr(_window(left, kind), operation)(right, pairwise=True)
    expected = getattr(_window(oracle_left, kind), operation)(oracle_right, pairwise=True)
    _result(actual, expected)


@pytest.mark.parametrize("operation", ["corr", "cov"])
@pytest.mark.parametrize("adjust", [False, True])
@pytest.mark.parametrize("ignore_na", [False, True])
@pytest.mark.parametrize("pairwise", [None, True, False])
def test_ewm_self_pairwise_defaults_and_controls(operation, adjust, ignore_na, pairwise):
    actual, _ = _cross_frames(fpd, "plain")
    expected, _ = _cross_frames(pd, "plain")
    _result(
        getattr(_window(actual, "ewm", adjust=adjust, ignore_na=ignore_na), operation)(
            pairwise=pairwise,
        ),
        getattr(_window(expected, "ewm", adjust=adjust, ignore_na=ignore_na), operation)(
            pairwise=pairwise,
        ),
    )


@pytest.mark.parametrize("kind", ["rolling", "expanding", "ewm"])
@pytest.mark.parametrize("operation", ["corr", "cov"])
def test_pairwise_numeric_only_and_nullable_inputs(kind, operation):
    def frames(module):
        return (
            module.DataFrame({
                "a": module.Series([1, None, 3, 4], dtype="Int64"),
                "b": [1.0, 2.0, 6.0, 4.0], "flag": [True, False, True, False],
                "text": ["x", "y", "z", "w"],
            }),
            module.DataFrame({
                "q": module.Series([1.0, 2.0, None, 8.0], dtype="Float64"),
                "flag": [False, True, True, False], "text": ["a", "b", "c", "d"],
            }),
        )

    actual, actual_other = frames(fpd)
    expected, expected_other = frames(pd)
    _result(
        getattr(_window(actual, kind), operation)(
            actual_other, pairwise=True, numeric_only=True,
        ),
        getattr(_window(expected, kind), operation)(
            expected_other, pairwise=True, numeric_only=True,
        ),
    )


@pytest.mark.parametrize("bias", [False, True])
@pytest.mark.parametrize("adjust", [False, True])
@pytest.mark.parametrize("ignore_na", [False, True])
def test_cross_ewm_covariance_options(bias, adjust, ignore_na):
    left, right = _cross_frames(fpd, "align")
    oracle_left, oracle_right = _cross_frames(pd, "align")
    _result(
        _window(left, "ewm", adjust=adjust, ignore_na=ignore_na).cov(
            right, pairwise=True, bias=bias,
        ),
        _window(oracle_left, "ewm", adjust=adjust, ignore_na=ignore_na).cov(
            oracle_right, pairwise=True, bias=bias,
        ),
    )


@pytest.mark.parametrize("kind", ["rolling", "expanding"])
@pytest.mark.parametrize("ddof", [0, 2])
def test_cross_window_covariance_ddof(kind, ddof):
    left, right = _cross_frames(fpd, "plain")
    oracle_left, oracle_right = _cross_frames(pd, "plain")
    _result(
        _window(left, kind).cov(right, pairwise=True, ddof=ddof),
        _window(oracle_left, kind).cov(oracle_right, pairwise=True, ddof=ddof),
    )


@pytest.mark.parametrize("kind", ["rolling", "expanding", "ewm"])
@pytest.mark.parametrize("operation", ["corr", "cov"])
def test_window_pair_names_do_not_collide(kind, operation):
    data = np.array([[1., 2., 6., 1.], [4., 8., 3., 2.], [9., 1., 7., 8.]])
    columns = ["a", "a__b", "b__c", "c"]
    actual = fpd.DataFrame(data, columns=columns)
    expected = pd.DataFrame(data, columns=columns)
    _result(
        getattr(_window(actual, kind), operation)(),
        getattr(_window(expected, kind), operation)(),
    )


@pytest.mark.parametrize("kind", ["rolling", "expanding", "ewm"])
@pytest.mark.parametrize("operation", ["corr", "cov"])
@pytest.mark.parametrize("case", ["duplicates", "multi_columns", "unsorted_multi_columns"])
def test_self_window_pairwise_axes(kind, operation, case):
    actual, _ = _cross_frames(fpd, case)
    expected, _ = _cross_frames(pd, case)
    _result(
        getattr(_window(actual, kind), operation)(pairwise=True),
        getattr(_window(expected, kind), operation)(pairwise=True),
    )


@pytest.mark.parametrize("kind", ["rolling", "expanding", "ewm"])
@pytest.mark.parametrize("rows", [0, 1])
def test_pairwise_empty_and_single_row_results(kind, rows):
    left = fpd.DataFrame({"a": np.ones(rows), "b": np.ones(rows)})
    right = fpd.DataFrame({"q": np.ones(rows)})
    oracle_left = pd.DataFrame({"a": np.ones(rows), "b": np.ones(rows)})
    oracle_right = pd.DataFrame({"q": np.ones(rows)})
    _result(
        _window(left, kind).cov(right, pairwise=True),
        _window(oracle_left, kind).cov(oracle_right, pairwise=True),
    )


@pytest.mark.parametrize("kind", ["rolling", "expanding", "ewm"])
@pytest.mark.parametrize("operation", ["corr", "cov"])
def test_pairwise_empty_axes_retain_names_and_unused_labels(kind, operation):
    def frames(module):
        return (
            module.DataFrame(
                np.empty((0, 2)), index=module.Index([], name="left_rows"),
                columns=module.Index(["a", "b"], name="left_columns"),
            ),
            module.DataFrame(
                np.empty((0, 2)), index=module.Index([], name="right_rows"),
                columns=module.Index(["z", "q"], name="right_columns"),
            ),
        )

    actual, actual_other = frames(fpd)
    expected, expected_other = frames(pd)
    _result(
        getattr(_window(actual, kind), operation)(actual_other, pairwise=True),
        getattr(_window(expected, kind), operation)(expected_other, pairwise=True),
    )


@pytest.mark.parametrize("kind", ["rolling", "expanding", "ewm"])
@pytest.mark.parametrize("case", ["duplicate", "multi_columns"])
def test_pairwise_empty_right_columns_match_pandas_errors(kind, case):
    for module in (pd, fpd):
        columns = (
            module.Index(["q", "q"])
            if case == "duplicate" else
            module.MultiIndex.from_tuples([("a", 1), ("b", 2)])
        )
        left = module.DataFrame(np.empty((0, 1)), columns=["a"])
        right = module.DataFrame(np.empty((0, 2)), columns=columns)
        error, message = (
            (ValueError, "Level values must be unique")
            if case == "duplicate" else
            (NotImplementedError, "isna is not defined for MultiIndex")
        )
        with pytest.raises(error, match=message):
            _window(left, kind).cov(right, pairwise=True)


@pytest.mark.parametrize("kind", ["rolling", "expanding", "ewm"])
def test_pairwise_empty_columns_match_pandas_error(kind):
    for module in (pd, fpd):
        frame = module.DataFrame(index=[0, 1])
        other = module.DataFrame({"a": [1.0, 2.0]})
        with pytest.raises(ValueError, match="No objects to concatenate"):
            _window(frame, kind).cov(other, pairwise=True)


def test_ewm_pairwise_timezone_rows():
    def frame(module):
        return module.DataFrame(
            {"a": [1., 2., 6.], "b": [7., 3., 8.]},
            index=module.date_range("2024-03-09", periods=3, tz="US/Eastern", name="day"),
        )

    _result(frame(fpd).ewm(alpha=.5).cov(), frame(pd).ewm(alpha=.5).cov())
