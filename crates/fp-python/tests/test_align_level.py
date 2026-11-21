"""MultiIndex-level alignment and broadcasting against live pandas 2.2.3.

The joined level catalog, tuple order, duplicates, value dtypes, and both
operands matter independently: a Cartesian product or a flattened index is
not an equivalent result. Regression coverage for u6p7i / n57tz.
"""

import numpy as np
import pandas as pd
import pytest

import frankenpandas as fpd


def _scalar(actual, expected):
    if isinstance(expected, tuple):
        assert isinstance(actual, tuple)
        assert len(actual) == len(expected)
        for a, e in zip(actual, expected):
            _scalar(a, e)
    elif expected is None:
        assert actual is None
    elif expected is pd.NA or expected is pd.NaT or bool(pd.isna(expected)):
        assert actual is None or type(actual).__name__ in {"NAType", "NaTType"} or bool(pd.isna(actual))
    elif isinstance(expected, (pd.Timestamp, pd.Timedelta)):
        assert str(actual) == str(expected)
    else:
        assert actual == expected


def _values(actual, expected):
    assert len(actual) == len(expected)
    for a, e in zip(actual, expected):
        _scalar(a, e)


def _index(actual, expected):
    assert type(actual).__name__ == type(expected).__name__
    assert list(actual.names) == list(expected.names)
    _values(actual.tolist(), expected.tolist())
    if isinstance(expected, pd.MultiIndex):
        assert actual.nlevels == expected.nlevels
        for a, e in zip(actual.levels, expected.levels):
            _values(a.tolist(), e.tolist())
            assert str(a.dtype) == str(e.dtype)
        for a, e in zip(actual.codes, expected.codes):
            assert list(a) == list(e)
    else:
        assert str(actual.dtype) == str(expected.dtype)


def _object(actual, expected):
    assert type(actual).__name__ == type(expected).__name__
    _index(actual.index, expected.index)
    if isinstance(expected, pd.Series):
        assert actual.name == expected.name
        assert str(actual.dtype) == str(expected.dtype)
        _values(actual.tolist(), expected.tolist())
    else:
        _index(actual.columns, expected.columns)
        assert actual.shape == expected.shape
        for position in range(expected.shape[1]):
            a, e = actual.iloc[:, position], expected.iloc[:, position]
            assert str(a.dtype) == str(e.dtype)
            _values(a.tolist(), e.tolist())


def _pair(actual, expected):
    assert len(actual) == len(expected) == 2
    for a, e in zip(actual, expected):
        _object(a, e)


def _result(run):
    try:
        expected = run(pd)
    except Exception as reference:
        with pytest.raises(type(reference)) as actual:
            run(fpd)
        assert str(actual.value) == str(reference)
    else:
        _pair(run(fpd), expected)


def _multi(module, duplicate=True):
    # Repeated tuples remain repeated, and the input order is deliberately
    # different from the selected level's sorted catalog.
    return module.MultiIndex.from_tuples(
        [("b", 2), ("a", 1), ("a", 2), ("b", 1), ("b", 1 if duplicate else 3)],
        names=["group", "row"],
    )


@pytest.mark.parametrize("join", ["outer", "inner", "left", "right"])
@pytest.mark.parametrize("reverse", [False, True])
@pytest.mark.parametrize("level", ["group", 0, -2])
def test_series_align_level_preserves_rows_and_catalog(join, reverse, level):
    def run(module):
        left = module.Series([1, 2, 3, 4, 5], index=_multi(module), name="left")
        right = module.Series([20, 30], index=module.Index(["c", "a"], name="other"), name="right")
        first, second = (right, left) if reverse else (left, right)
        return first.align(second, join=join, level=level)

    _pair(run(fpd), run(pd))


@pytest.mark.parametrize("dtype,data", [
    ("int64", [10, 20]), ("int8", [10, 20]),
    ("float32", [1.5, np.nan]), ("Int64", [10, None]),
    ("boolean", [True, None]), ("string", ["x", None]),
    ("object", [None, "x"]),
    ("datetime64[ns]", ["2024-01-01", None]),
    ("timedelta64[ns]", [1, None]),
])
def test_series_align_level_preserves_value_dtype(dtype, data):
    def run(module):
        left = module.Series([1, 2, 3, 4, 5], index=_multi(module), name="left")
        right = module.Series(data, index=["a", "c"], dtype=dtype, name="right")
        return left.align(right, level="group")

    _pair(run(fpd), run(pd))


@pytest.mark.parametrize("join", ["outer", "inner", "left", "right"])
@pytest.mark.parametrize("reverse", [False, True])
def test_series_align_on_inner_numeric_level(join, reverse):
    def run(module):
        left = module.Series([1, 2, 3, 4, 5], index=_multi(module), name="left")
        right = module.Series([10, 30], index=[2.0, 3.0], name="right")
        a, b = (right, left) if reverse else (left, right)
        return a.align(b, join=join, level="row")

    _pair(run(fpd), run(pd))


@pytest.mark.parametrize("join", ["outer", "inner", "left", "right"])
@pytest.mark.parametrize("keys", [[2, 3], [1, 2], [3, 4]])
@pytest.mark.parametrize("level_dtype,flat_dtype", [
    ("int8", "int8"), ("float32", "float32"),
    ("int8", "float32"), ("int32", "float32"),
    ("int64", "float64"), ("uint64", "uint64"),
])
def test_numeric_level_catalog_dtype_negotiation(join, keys, level_dtype, flat_dtype):
    def run(module):
        catalog = module.Index([1, 2], dtype=level_dtype)
        index = module.MultiIndex.from_product([catalog, ["x"]], names=["number", "row"])
        left = module.Series([10, 20], index=index, name="left")
        right = module.Series([30, 40], index=module.Index(keys, dtype=flat_dtype), name="right")
        return left.align(right, join=join, level="number")

    _pair(run(fpd), run(pd))


@pytest.mark.parametrize("join", ["outer", "inner", "left", "right"])
@pytest.mark.parametrize("axis", [0, 1, None])
@pytest.mark.parametrize("reverse", [False, True])
@pytest.mark.parametrize("fill", [None, 0])
def test_dataframe_align_level_on_rows_or_columns(join, axis, reverse, fill):
    def run(module):
        left = module.DataFrame(
            {"value": [1, 2, 3, 4, 5], "shared": [1.0, np.nan, 3.0, 4.0, 5.0]},
            index=_multi(module),
        )
        right = module.DataFrame({"shared": [10, 20], "extra": [30, 40]}, index=["c", "a"])
        if axis == 1:
            left, right = left.T, right.T
        a, b = (right, left) if reverse else (left, right)
        return a.align(b, join=join, axis=axis, level="group", fill_value=fill)

    _pair(run(fpd), run(pd))


@pytest.mark.parametrize("join", ["outer", "inner", "left", "right"])
@pytest.mark.parametrize("axis", [0, 1])
@pytest.mark.parametrize("multi_on_frame", [False, True])
@pytest.mark.parametrize("fill", [None, 0])
@pytest.mark.parametrize("duplicate", [False, True])
def test_frame_series_level_broadcast(join, axis, multi_on_frame, fill, duplicate):
    def run(module):
        if multi_on_frame:
            frame = module.DataFrame({"v": [1, 2, 3, 4, 5]}, index=_multi(module, duplicate))
            series = module.Series([10, 20], index=["c", "a"], name="weights")
        else:
            frame = module.DataFrame({"v": [10, 20]}, index=["c", "a"])
            series = module.Series([1, 2, 3, 4, 5], index=_multi(module, duplicate), name="weights")
        if axis == 1:
            frame = frame.T
        return frame.align(series, join=join, axis=axis, level="group", fill_value=fill)

    # pandas' frame/Series path rejects some duplicate-axis reindexers and
    # column joins that would reindex one MultiIndex against another.
    _result(run)


@pytest.mark.parametrize("join", ["outer", "inner", "left", "right"])
@pytest.mark.parametrize("fill", [None, 0])
def test_series_align_dataframe_by_level(join, fill):
    def run(module):
        series = module.Series([10.0, np.nan], index=["c", "a"], name="value")
        frame = module.DataFrame({"v": [1.0, 2.0, np.nan, 4.0, 5.0]}, index=_multi(module))
        return series.align(frame, join=join, level="group", fill_value=fill)

    _pair(run(fpd), run(pd))


@pytest.mark.parametrize("join", ["outer", "inner", "left", "right"])
def test_level_alignment_preserves_missing_unselected_codes(join):
    def run(module):
        index = module.MultiIndex.from_tuples([("a", 1), ("b", None), ("a", None)], names=["group", "row"])
        left = module.Series([1, 2, 3], index=index)
        right = module.Series([4], index=["a"])
        return left.align(right, join=join, level="group")

    _pair(run(fpd), run(pd))


@pytest.mark.parametrize("flat", [["a", "b"], ["b", "a"], ["a"]])
def test_left_alignment_handles_missing_selected_level_codes(flat):
    def run(module):
        index = module.MultiIndex.from_tuples([("a", 1), (None, 2), ("b", 3)], names=["group", "row"])
        return module.Series([1, 2, 3], index=index).align(
            module.Series(range(len(flat)), index=flat), join="left", level="group",
        )

    _pair(run(fpd), run(pd))


@pytest.mark.parametrize("join", ["outer", "inner", "left", "right"])
@pytest.mark.parametrize("empty_side", ["multi", "flat", "disjoint"])
def test_empty_and_nonmatching_level_alignment(join, empty_side):
    def run(module):
        index = _multi(module)
        left = module.Series([1, 2, 3, 4, 5], index=index, name="left")
        right = module.Series([10], index=["z"], name="right", dtype="int64")
        if empty_side == "multi":
            left = left.iloc[:0]
        elif empty_side == "flat":
            right = right.iloc[:0]
        return left.align(right, join=join, level="group")

    _pair(run(fpd), run(pd))


@pytest.mark.parametrize("case", ["both_multi", "duplicate_flat", "bad_level", "bad_number", "duplicate_names"])
def test_level_alignment_raises_pandas_errors(case):
    def run(module):
        index = _multi(module)
        if case == "duplicate_names":
            index = index.set_names(["group", "group"])
        left = module.Series([1, 2, 3, 4, 5], index=index)
        right = module.Series([10, 20], index=["a", "a"] if case == "duplicate_flat" else ["a", "b"])
        if case == "both_multi":
            right = left.iloc[:2]
        level = "absent" if case == "bad_level" else 5 if case == "bad_number" else "group"
        return left.align(right, level=level)

    with pytest.raises(Exception) as reference:
        run(pd)
    with pytest.raises(Exception) as actual:
        run(fpd)
    assert type(actual.value).__name__ == type(reference.value).__name__
    assert str(actual.value) == str(reference.value)


def test_equal_multiindexes_and_flat_indexes_ignore_level():
    for equal_multi in [False, True]:
        def run(module):
            index = _multi(module) if equal_multi else module.Index(["b", "a"])
            left = module.Series(range(len(index)), index=index, name="left")
            right = module.Series(range(len(index)), index=index, name="right")
            return left.align(right, level="does_not_exist")

        _pair(run(fpd), run(pd))


def test_numeric_level_name_precedes_position():
    def run(module):
        index = _multi(module).set_names([1, 2])
        return module.Series([1, 2, 3, 4, 5], index=index).align(
            module.Series([10], index=["a"]), level=1,
        )

    _pair(run(fpd), run(pd))


def test_alignment_preserves_datetime_level_timezone():
    def run(module):
        dates = module.date_range("2024-03-09", periods=3, tz="America/New_York")
        index = module.MultiIndex.from_product([dates, [1, 2]], names=["day", "slot"])
        left = module.Series(range(6), index=index)
        right = module.Series([10, 20], index=dates[::2])
        return left.align(right, level="day")

    _pair(run(fpd), run(pd))
