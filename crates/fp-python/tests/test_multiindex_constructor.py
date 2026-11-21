"""Declared MultiIndex catalogs are observable, including unused labels."""

import numpy as np
import pandas as pd
import pytest

import frankenpandas as fpd


def assert_multiindex(actual, expected):
    assert actual.nlevels == expected.nlevels
    assert actual.shape == expected.shape
    assert list(actual.names) == list(expected.names)
    assert actual.levshape == expected.levshape
    np.testing.assert_equal(actual.tolist(), expected.tolist())
    for got, want in zip(actual.levels, expected.levels):
        np.testing.assert_equal(got.tolist(), want.tolist())
        assert str(got.dtype) == str(want.dtype)
        assert got.name == want.name
    for got, want in zip(actual.codes, expected.codes):
        np.testing.assert_array_equal(got, want)
        assert got.dtype == want.dtype
        assert not got.flags.writeable


@pytest.mark.parametrize("codes", [[[], []], [[1, 2, 1], [1, 2, 2]]])
@pytest.mark.parametrize("names", [None, ["r", "c"], [0, ("column", 1)]])
def test_declared_catalogs_survive_constructor_and_series_index(codes, names):
    levels = [["z", "b", "a", "unused"], [4, 2, 1]]
    actual = fpd.MultiIndex(levels=levels, codes=codes, names=names)
    expected = pd.MultiIndex(levels=levels, codes=codes, names=names)
    assert_multiindex(actual, expected)
    values = np.arange(len(expected))
    assert_multiindex(
        fpd.Series(values, index=actual).index,
        pd.Series(values, index=expected).index,
    )


@pytest.mark.parametrize(
    "levels,codes",
    [
        ([["b", "a", "unused"], [2, 1]], [[0, -1, 1], [-1, 1, 0]]),
        ([[], []], [[-1], [-1]]),
        ([["b", "a"]], [[1.9, 0.0]]),
        ([["b", "a"]], [["1", "0"]]),
        ([np.array([3, 1, 2], dtype="int16")], [[1, 0]]),
        ([np.array([1.5, 0.5], dtype="float32")], [[1, 0]]),
    ],
)
def test_declared_codes_missing_values_and_numeric_widths(levels, codes):
    assert_multiindex(
        fpd.MultiIndex(levels=levels, codes=codes),
        pd.MultiIndex(levels=levels, codes=codes),
    )


def test_declared_constructor_name_alias_and_object_dtype():
    def build(mod):
        level = mod.Index(["z", "a", "unused"], name="old")
        return mod.MultiIndex(levels=[level], codes=[[1, 0]], name=["new"], dtype=object)

    assert_multiindex(build(fpd), build(pd))


@pytest.mark.parametrize(
    "dtype",
    ["int64", "float64", "bool", "datetime64[ns]", "timedelta64[ns]", "int8", "float32", "object"],
)
def test_empty_declared_catalog_preserves_its_dtype(dtype):
    def build(mod):
        level = mod.Index([], dtype=dtype)
        return mod.MultiIndex(levels=[level], codes=[[]], names=["typed"])

    actual, expected = build(fpd), build(pd)
    assert_multiindex(actual, expected)
    assert_multiindex(actual[:0], expected[:0])
    assert_multiindex(
        fpd.Series([], index=actual, dtype="float64").index,
        pd.Series([], index=expected, dtype="float64").index,
    )


def test_numeric_labels_keep_an_explicit_object_catalog():
    def build(mod):
        level = mod.Index([1, 2], dtype="object")
        return mod.MultiIndex(levels=[level], codes=[[1, 0]], names=["objects"])

    actual, expected = build(fpd), build(pd)
    assert_multiindex(actual, expected)
    assert_multiindex(actual[:0], expected[:0])


def test_declared_constructor_copy_owns_input_catalogs_and_codes():
    levels, codes = [["z", "a", "unused"]], [[1, 0]]
    actual = fpd.MultiIndex(levels=levels, codes=codes, copy=True)
    expected = pd.MultiIndex(levels=levels, codes=codes, copy=True)
    levels[0][0] = "changed"
    codes[0][0] = 0
    assert_multiindex(actual, expected)


@pytest.mark.parametrize(
    "kwargs",
    [
        {"levels": [], "codes": []},
        {"levels": [["a"]], "codes": [[], []]},
        {"levels": [["a"], [1]], "codes": [[0], []]},
        {"levels": [["a"]], "codes": [[-2]]},
        {"levels": [["a"]], "codes": [[1]]},
        {"levels": [["a", "a"]], "codes": [[0]]},
        {"levels": [[True, 1]], "codes": [[0]]},
        {"levels": [np.array([[1, 2]])], "codes": [[0]]},
        {"levels": [[1]], "codes": [[0]], "names": []},
        {"levels": [[1]], "codes": [[0]], "names": "row"},
    ],
)
def test_declared_constructor_rejects_invalid_structure(kwargs):
    for mod in (pd, fpd):
        with pytest.raises(ValueError):
            mod.MultiIndex(**kwargs)


@pytest.mark.parametrize("kwargs", [{}, {"levels": [[1]]}, {"codes": [[0]]}])
def test_declared_constructor_requires_levels_and_codes(kwargs):
    for mod in (pd, fpd):
        with pytest.raises(TypeError):
            mod.MultiIndex(**kwargs)


def test_declared_constructor_rejects_multidimensional_codes():
    with pytest.raises(ValueError, match="one-dimensional"):
        fpd.MultiIndex(levels=[[1]], codes=[np.array([[0]])])


@pytest.mark.parametrize("option", [{"sortorder": 0}, {"verify_integrity": False}])
def test_declared_constructor_refuses_unimplemented_options(option):
    with pytest.raises(NotImplementedError):
        fpd.MultiIndex(levels=[[1]], codes=[[0]], **option)


def test_declared_constructor_refuses_missing_catalog_labels_explicitly():
    with pytest.raises(NotImplementedError):
        fpd.MultiIndex(levels=[[None, "a"]], codes=[[1]])
