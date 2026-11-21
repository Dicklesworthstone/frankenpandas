"""Sparse public workflows against pandas 2.2.3 (o1qso, at95d).

The extension must be built: an unavailable module fails collection instead
of turning missing sparse functionality into skipped parity evidence.
"""

import pickle

import numpy as np
import pandas as pd
import pytest

import frankenpandas as fpd


@pytest.mark.parametrize(
    "subtype,fill,values",
    [
        ("int64", 0, [0, 2**53 + 1, 0, 2**53 + 3]),
        ("int64", 7, [7, 1, 7, 3]),
        ("int64", np.nan, [np.nan, 1, 2]),
        ("float32", 0, [0, np.nan, 1.25, 2.5]),
        ("float32", np.nan, [np.nan, 1.25, np.nan, 2.5]),
        ("float64", 0, [0, np.nan, 2]),
        ("float64", np.nan, [np.nan, 1, np.nan]),
        ("bool", False, [False, True, False]),
        ("bool", True, [True, True]),
        ("object", "", ["", "x", "", "y"]),
        ("int64", 0, []),
        ("float32", np.nan, [np.nan, np.nan]),
        ("float32", 0, [np.nan, np.nan]),
    ],
)
@pytest.mark.parametrize("op", ["sum", "mean", "min", "max", "any", "all"])
@pytest.mark.parametrize("skipna", [False, True])
def test_sparse_reductions_preserve_compressed_semantics(subtype, fill, values, op, skipna):
    actual = fpd.Series(values, dtype=fpd.SparseDtype(subtype, fill))
    expected = pd.Series(values, dtype=pd.SparseDtype(subtype, fill))
    with np.errstate(invalid="ignore", divide="ignore"):
        try:
            want = getattr(expected, op)(skipna=skipna)
        except TypeError:
            with pytest.raises(TypeError):
                getattr(actual, op)(skipna=skipna)
            return
        got = getattr(actual, op)(skipna=skipna)
    if pd.isna(want):
        assert pd.isna(got)
    else:
        assert got == want
    assert type(got) is type(want)


@pytest.mark.parametrize("subtype,fill,values", [
    ("float32", 0, [0, np.nan, 0]),
    ("float32", np.nan, [np.nan, np.nan]),
    ("object", "", [None, "x"]),
    ("object", "", [None, None]),
])
@pytest.mark.parametrize("operation", ["any", "all"])
@pytest.mark.parametrize("skipna", [False, True])
def test_sparse_logical_missing_values_follow_pinned_pandas(subtype, fill, values, operation, skipna):
    actual = fpd.Series(values, dtype=fpd.SparseDtype(subtype, fill))
    expected = pd.Series(values, dtype=pd.SparseDtype(subtype, fill))
    got = getattr(actual, operation)(skipna=skipna)
    want = getattr(expected, operation)(skipna=skipna)
    assert got == want
    assert type(got) is type(want)


@pytest.mark.parametrize("subtype,fill", [("int64", 0), ("float32", np.nan)])
@pytest.mark.parametrize("min_count", [0, 1, 3, 4])
def test_sparse_sum_min_count(subtype, fill, min_count):
    values = [fill, 1, fill]
    actual = fpd.Series(values, dtype=fpd.SparseDtype(subtype, fill)).sum(min_count=min_count)
    expected = pd.Series(values, dtype=pd.SparseDtype(subtype, fill)).sum(min_count=min_count)
    if pd.isna(expected):
        assert pd.isna(actual)
    else:
        assert actual == expected
    assert type(actual) is type(expected)


@pytest.mark.parametrize("op", ["median", "std", "var", "prod", "skew", "kurt", "sem"])
@pytest.mark.parametrize("numeric_only", [False, True])
def test_sparse_unsupported_reductions_match_pandas(op, numeric_only):
    for mod in (pd, fpd):
        series = mod.Series([0, 1, 2], dtype="Sparse[int]")
        with pytest.raises(TypeError, match=f"cannot perform {op} with type Sparse"):
            getattr(series, op)(numeric_only=numeric_only)


def assert_sparse_values(actual, expected):
    assert actual.shape == expected.shape
    if expected.dtype != object:
        np.testing.assert_array_equal(actual, expected)
        return
    # NumPy's object-array assertion considers independently created NaNs
    # unequal. Keep None/NaN/NaT distinct while comparing their missingness.
    for got, want in zip(actual.flat, expected.flat, strict=True):
        if pd.isna(want):
            assert type(got) is type(want)
            assert pd.isna(got)
        else:
            assert got == want


def assert_sparse_series(actual, expected):
    assert str(actual.dtype) == str(expected.dtype)
    assert str(actual.dtype.subtype) == str(expected.dtype.subtype)
    assert actual.dtype.kind == expected.dtype.kind
    assert actual.index.tolist() == expected.index.tolist()
    assert actual.index.name == expected.index.name
    if isinstance(expected.name, float) and np.isnan(expected.name):
        assert isinstance(actual.name, float) and np.isnan(actual.name)
    else:
        assert actual.name == expected.name
    assert_sparse_values(actual.to_numpy(), expected.to_numpy())
    assert actual.to_numpy().dtype == expected.to_numpy().dtype
    assert_sparse_values(actual.sparse.sp_values, expected.sparse.sp_values)
    assert actual.sparse.sp_values.dtype == expected.sparse.sp_values.dtype
    assert actual.sparse.npoints == expected.sparse.npoints
    if len(expected):
        assert actual.sparse.density == expected.sparse.density
    else:
        with pytest.raises(ZeroDivisionError):
            actual.sparse.density
    if pd.isna(expected.sparse.fill_value):
        assert pd.isna(actual.sparse.fill_value)
    else:
        assert actual.sparse.fill_value == expected.sparse.fill_value
        assert type(actual.sparse.fill_value) is type(expected.sparse.fill_value)


@pytest.mark.parametrize(
    "values,dtype",
    [
        ([0, 0, 1, -2], "Sparse[int]"),
        ([0, 0, 1, -2], "Sparse[int64, 0]"),
        ([0, 0, 127, -128], "Sparse[int8]"),
        ([0, 1, 255], "Sparse[uint8]"),
        ([np.nan, 1.25, np.nan, -3.0], "Sparse[float]"),
        ([np.nan, 1.25, np.nan, -3.0], "Sparse[float32]"),
        ([False, True, False], "Sparse[bool]"),
        (["x", None, "y"], "Sparse[object]"),
        ([], "Sparse[int]"),
        ([], "Sparse[float]"),
        ([0, 1, 2], "Sparse"),
    ],
)
def test_sparse_series_dtype_strings(values, dtype):
    actual = fpd.Series(values, dtype=dtype, name="value")
    expected = pd.Series(values, dtype=dtype, name="value")
    assert_sparse_series(actual, expected)
    assert isinstance(actual.array, fpd.arrays.SparseArray)
    assert isinstance(actual.values, fpd.arrays.SparseArray)


@pytest.mark.parametrize(
    "subtype,fill,values",
    [
        ("int64", 7, [7, 1, 7, 2]),
        ("float64", 0, [0, 1.5, 0, np.nan]),
        ("float64", 0.0, [0, 1.5, 0, np.nan]),
        ("float32", 0, [0, 1.5, 0, 2.5]),
        ("bool", True, [True, False, True]),
        ("object", "", ["", "x", "", None]),
        ("int64", np.nan, [2**53 + 1, None, 2**53 + 3]),
    ],
)
def test_sparse_custom_fill_and_exact_values(subtype, fill, values):
    actual = fpd.Series(values, dtype=fpd.SparseDtype(subtype, fill), name="x")
    expected = pd.Series(values, dtype=pd.SparseDtype(subtype, fill), name="x")
    assert_sparse_series(actual, expected)
    for got, want in zip(actual.tolist(), expected.tolist()):
        if not pd.isna(want):
            assert got == want
            assert type(got) is type(want)


@pytest.mark.parametrize("constructor", ["series", "array"])
@pytest.mark.parametrize("source", ["list", "tuple", "object_array", "object_series", "inferred_series"])
@pytest.mark.parametrize("subtype", ["int64", "object"])
def test_sparse_constructor_respects_initial_source_dtype(constructor, source, subtype):
    values = [2**53 + 1, None, 2**53 + 3]

    def build(mod):
        if source == "tuple":
            data = tuple(values)
        elif source == "object_array":
            data = np.array(values, dtype=object)
        elif source == "object_series":
            data = mod.Series(values, dtype=object)
        elif source == "inferred_series":
            data = mod.Series(values)
        else:
            data = values
        dtype = mod.SparseDtype(subtype, np.nan)
        if constructor == "array":
            return mod.Series(mod.arrays.SparseArray(data, dtype=dtype))
        return mod.Series(data, dtype=dtype)

    assert_sparse_series(build(fpd), build(pd))


@pytest.mark.parametrize("subtype", ["int64", "int8", "uint16", "float64", "float32", "bool", "object"])
def test_sparse_dtype_defaults(subtype):
    actual, expected = fpd.SparseDtype(subtype), pd.SparseDtype(subtype)
    assert repr(actual) == repr(expected)
    assert str(actual.subtype) == str(expected.subtype)
    assert actual.kind == expected.kind
    assert actual == fpd.SparseDtype(subtype)
    assert actual == str(expected)
    assert pickle.loads(pickle.dumps(actual)) == actual


@pytest.mark.parametrize("dtype", ["Sparse[int64, 7]", "Sparse[float64, 0]", "Sparse[bool, True]"])
def test_custom_fill_string_refused_like_pandas(dtype):
    with pytest.raises(TypeError):
        pd.Series([0, 1], dtype=dtype)
    with pytest.raises(TypeError):
        fpd.Series([0, 1], dtype=dtype)


@pytest.mark.parametrize("values", [[0.0, np.nan, 1.0], [0.0, np.inf], [None, 0, 1]])
def test_sparse_integer_rejects_nonfinite_stored_values(values):
    with pytest.raises(pd.errors.IntCastingNaNError):
        pd.Series(values, dtype="Sparse[int]")
    with pytest.raises(fpd.errors.IntCastingNaNError):
        fpd.Series(values, dtype="Sparse[int]")


@pytest.mark.parametrize("values", [[0, 0, 1], [False, True, False], [1.5, None, 2.5], [], np.array([0, 2, 0], dtype="int8")])
def test_sparse_array_inference_and_series(values):
    actual_array = fpd.arrays.SparseArray(values)
    expected_array = pd.arrays.SparseArray(values)
    assert str(actual_array.dtype) == str(expected_array.dtype)
    np.testing.assert_array_equal(actual_array.to_dense(), expected_array.to_dense())
    assert actual_array.to_dense().dtype == expected_array.to_dense().dtype
    assert_sparse_series(fpd.Series(actual_array), pd.Series(expected_array))


def test_sparse_cast_preserves_explicit_positions_and_index():
    actual = fpd.Series([0.1, 0, 2.7], index=fpd.Index(["a", "b", "c"], name="row"), name="x").astype("Sparse[int]")
    expected = pd.Series([0.1, 0, 2.7], index=pd.Index(["a", "b", "c"], name="row"), name="x").astype("Sparse[int]")
    assert_sparse_series(actual, expected)
    assert actual.sparse.npoints == 2  # the 0.1 cast to 0 stays explicitly stored
    actual = actual.astype(fpd.SparseDtype("float64", np.nan))
    expected = expected.astype(pd.SparseDtype("float64", np.nan))
    assert_sparse_series(actual, expected)


@pytest.mark.parametrize("operation", [lambda s: s + 2, lambda s: s * 0, lambda s: s - 1, lambda s: s > 0, lambda s: abs(s), lambda s: s.iloc[::-1]])
def test_sparse_arithmetic_and_gathers(operation):
    actual = operation(fpd.Series([0, -2, 0, 3], dtype="Sparse[int]"))
    expected = operation(pd.Series([0, -2, 0, 3], dtype="Sparse[int]"))
    assert_sparse_series(actual, expected)


@pytest.mark.parametrize("dtype", ["Sparse[int64]", "Sparse[int8]", "Sparse[float32]"])
@pytest.mark.parametrize("operation", [lambda s: s < 2, lambda s: s >= 1, lambda s: s == 2, lambda s: s != 2])
def test_sparse_scalar_comparisons_include_implicit_fill(dtype, operation):
    actual = operation(fpd.Series([0, -2, 0, 2], dtype=dtype))
    expected = operation(pd.Series([0, -2, 0, 2], dtype=dtype))
    assert_sparse_series(actual, expected)


@pytest.mark.parametrize("operation", [lambda s: s == 1.1, lambda s: s < 1.1])
def test_sparse_float32_scalar_comparison_rounding(operation):
    actual = operation(fpd.Series([0, 1.1, 0, 2.2], dtype=fpd.SparseDtype("float32", 0)))
    expected = operation(pd.Series([0, 1.1, 0, 2.2], dtype=pd.SparseDtype("float32", 0)))
    assert_sparse_series(actual, expected)


def test_sparse_array_cast_and_numpy_protocol():
    actual = fpd.arrays.SparseArray([0, 2, 0], dtype="int8")
    expected = pd.arrays.SparseArray([0, 2, 0], dtype="int8")
    for got, want in [(actual, expected), (actual + 2, expected + 2), (actual.astype("Sparse[float]"), expected.astype("Sparse[float]"))]:
        assert str(got.dtype) == str(want.dtype)
        np.testing.assert_array_equal(np.asarray(got), np.asarray(want))
        assert np.asarray(got).dtype == np.asarray(want).dtype
    np.testing.assert_array_equal(actual.astype("float64"), expected.astype("float64"))


@pytest.mark.parametrize("values,fill", [([7, 2, 7, 2], 7), ([0.1, 0, 2.7, 0], 0)])
@pytest.mark.parametrize(
    "operation",
    [
        lambda a: a.unique(),
        lambda a: a.take([3, 2, 0, 2]),
        lambda a: a.take([-1, 0, 1], allow_fill=True),
        lambda a: a.take([-1, 0, 1], allow_fill=True, fill_value=0),
        lambda a: a.take([0, 2], allow_fill=True),
        lambda a: a.take([]),
    ],
)
def test_sparse_array_take_and_unique_preserve_descriptor(values, fill, operation):
    actual = operation(fpd.arrays.SparseArray(values, dtype=fpd.SparseDtype("int8", fill)))
    expected = operation(pd.arrays.SparseArray(values, dtype=pd.SparseDtype("int8", fill)))
    assert_sparse_series(fpd.Series(actual), pd.Series(expected))
    np.testing.assert_array_equal(actual.sp_index.indices, expected.sp_index.indices)


def test_sparse_series_unique_returns_sparse_array():
    actual = fpd.Series([7, 2, 7, 3, 2], dtype=fpd.SparseDtype("int16", 7)).unique()
    expected = pd.Series([7, 2, 7, 3, 2], dtype=pd.SparseDtype("int16", 7)).unique()
    assert isinstance(actual, fpd.arrays.SparseArray)
    assert_sparse_series(fpd.Series(actual), pd.Series(expected))


@pytest.mark.parametrize("subtype,fill", [("int8", np.int8(0)), ("float32", np.float32(0)), ("bool", np.bool_(False))])
def test_sparse_numpy_fill_metadata_roundtrips(subtype, fill):
    dtype = fpd.SparseDtype(subtype, fill)
    expected_dtype = pd.SparseDtype(subtype, fill)
    restored = pickle.loads(pickle.dumps(dtype))
    assert str(restored) == str(expected_dtype)
    assert type(restored.fill_value) is type(expected_dtype.fill_value)
    actual = fpd.Series([0, 1, 0], dtype=restored)
    expected = pd.Series([0, 1, 0], dtype=expected_dtype)
    assert_sparse_series(actual, expected)
    assert_sparse_series(actual.iloc[::-1], expected.iloc[::-1])


@pytest.mark.parametrize("subtype", ["int8", "int64", "float32", "float64"])
def test_sparse_arithmetic_fill_dtype_differs_from_stored_subtype(subtype):
    actual = fpd.Series([0, 1, 0], dtype=fpd.SparseDtype(subtype, 0))
    expected = pd.Series([0, 1, 0], dtype=pd.SparseDtype(subtype, 0))
    assert_sparse_series(actual + 2, expected + 2)
    assert_sparse_series(actual * 0, expected * 0)
    assert_sparse_series(actual + actual, expected + expected)
    assert_sparse_series(abs(actual + 2), abs(expected + 2))


def assert_sparse_frame(actual, expected):
    assert actual.shape == expected.shape
    pd.testing.assert_index_equal(
        pd.Index(actual.columns.tolist(), name=actual.columns.name), expected.columns, exact=False
    )
    assert actual.columns.name == expected.columns.name
    assert actual.index.tolist() == expected.index.tolist()
    assert actual.index.name == expected.index.name
    assert actual.sparse.density == expected.sparse.density
    actual_dense = actual.sparse.to_dense()
    # DISC-035: pandas 2.2.3 frame densification overwrites distinct physical
    # columns that share a label. For that case only, compare against each
    # pandas Series' dense values; these assertions protect data integrity,
    # rather than claiming whole-frame parity with the upstream data loss.
    expected_dense = None if expected.columns.has_duplicates else expected.sparse.to_dense()
    for position in range(expected.shape[1]):
        assert_sparse_series(actual.iloc[:, position], expected.iloc[:, position])
        got = actual_dense.iloc[:, position]
        want = (
            expected.iloc[:, position].sparse.to_dense()
            if expected_dense is None
            else expected_dense.iloc[:, position]
        )
        assert str(got.dtype) == str(want.dtype)
        assert got.index.tolist() == want.index.tolist()
        if isinstance(want.name, float) and np.isnan(want.name):
            assert isinstance(got.name, float) and np.isnan(got.name)
        else:
            assert got.name == want.name
        assert_sparse_values(got.to_numpy(), want.to_numpy())


def test_pandas_223_duplicate_sparse_to_dense_bug_witness_disc035():
    assert pd.__version__ == "2.2.3", "Review DISC-035 when the pinned pandas oracle changes"
    values = [[0, 1], [2, 0], [0, 3]]
    sparse = pd.DataFrame(values, columns=["same", "same"]).astype("Sparse[int]")
    np.testing.assert_array_equal(sparse.to_numpy(), values)
    np.testing.assert_array_equal(
        np.column_stack([sparse.iloc[:, position].sparse.to_dense() for position in range(2)]),
        values,
    )
    # Negative control: the frame accessor duplicates the last column even
    # though both physical sparse columns still contain the original data.
    np.testing.assert_array_equal(sparse.sparse.to_dense().to_numpy(), [[1, 1], [0, 0], [3, 3]])


@pytest.mark.parametrize("labels", ["text", "numeric", "multiindex"])
def test_sparse_duplicate_to_dense_preserves_physical_columns_disc035(labels):
    values = [[0, 1], [2, 0], [0, 3]]
    columns = ["same", "same"] if labels == "text" else [7, 7]
    if labels == "multiindex":
        columns = fpd.MultiIndex.from_tuples([("kind", "same"), ("kind", "same")])
    sparse = fpd.DataFrame(values, columns=columns, index=fpd.Index([3, 1, 2], name="row"))
    sparse = sparse.astype("Sparse[int]")
    dense = sparse.sparse.to_dense()
    assert dense.columns.tolist() == sparse.columns.tolist()
    assert dense.index.tolist() == [3, 1, 2]
    assert dense.index.name == "row"
    assert [str(dtype) for dtype in dense.dtypes] == ["int64", "int64"]
    np.testing.assert_array_equal(dense.to_numpy(), values)
    np.testing.assert_array_equal(sparse.to_numpy(), values)


@pytest.mark.parametrize("dtype", ["Sparse[int]", "Sparse[float32]", "Sparse[bool]"])
@pytest.mark.parametrize("method", ["constructor", "astype", "array_columns"])
def test_sparse_dataframe_construction_and_dense_conversion(dtype, method):
    def build(mod):
        values = {"a": [0, 0, 1, 0], "b": [1, 0, 0, 1]}
        index = mod.Index(["w", "x", "y", "z"], name="row")
        if method == "array_columns":
            values = {key: mod.arrays.SparseArray(data, dtype=dtype) for key, data in values.items()}
            result = mod.DataFrame(values, index=index)
        elif method == "astype":
            result = mod.DataFrame(values, index=index).astype(dtype)
        else:
            result = mod.DataFrame(values, index=index, dtype=dtype)
        result.columns = mod.Index(result.columns.tolist(), name="feature")
        return result

    assert_sparse_frame(build(fpd), build(pd))


def test_sparse_dataframe_custom_dtypes_and_repeated_columns():
    def build(mod):
        frame = mod.DataFrame({"a": [7, 2, 7], "b": [0, 1.5, 0]})
        return frame.astype({"a": mod.SparseDtype("int16", 7), "b": mod.SparseDtype("float32", 0)})

    assert_sparse_frame(build(fpd), build(pd))
    assert_sparse_frame(
        fpd.DataFrame([[0, 1], [2, 0], [0, 3]], columns=["x", "x"]).astype("Sparse[int]"),
        pd.DataFrame([[0, 1], [2, 0], [0, 3]], columns=["x", "x"]).astype("Sparse[int]"),
    )


@pytest.mark.parametrize("labels", ["text", "numeric", "multiindex"])
def test_sparse_dataframe_mapping_casts_repeated_columns_by_position(labels):
    def build(mod):
        key = "x" if labels == "text" else 7
        columns = [key, key]
        if labels == "multiindex":
            key = ("kind", "x")
            columns = mod.MultiIndex.from_tuples([key, key])
        frame = mod.DataFrame([[0, 5], [2, 0]], columns=columns, index=mod.Index([3, 1], name="row"))
        return frame.astype({key: mod.SparseDtype("int16", 0)})

    assert_sparse_frame(build(fpd), build(pd))


@pytest.mark.parametrize("mapping", [False, True])
def test_sparse_dataframe_cast_ignore_keeps_successful_column_casts(mapping):
    def build(mod):
        frame = mod.DataFrame({"good": [0, 1], "bad": ["bad", "values"], "other": [5, 6]})
        dtype = mod.SparseDtype("int64", 0)
        spec = {"good": dtype, "bad": dtype} if mapping else dtype
        return frame.astype(spec, errors="ignore")

    actual, expected = build(fpd), build(pd)
    assert actual.columns.tolist() == expected.columns.tolist()
    assert [str(dtype) for dtype in actual.dtypes] == [str(dtype) for dtype in expected.dtypes]
    np.testing.assert_array_equal(actual.to_numpy(), expected.to_numpy())
    assert_sparse_series(actual["good"], expected["good"])
    assert actual["bad"].tolist() == expected["bad"].tolist() == ["bad", "values"]


def test_sparse_dataframe_mapping_combines_sparse_and_dense_casts():
    def build(mod):
        frame = mod.DataFrame([[0, 5, "1.5"], [2, 0, "2.5"]], columns=["x", "x", "other"])
        return frame.astype({"x": mod.SparseDtype("int16", 0), "other": "float32"})

    actual, expected = build(fpd), build(pd)
    assert [str(dtype) for dtype in actual.dtypes] == [str(dtype) for dtype in expected.dtypes]
    for position in range(2):
        assert_sparse_series(actual.iloc[:, position], expected.iloc[:, position])
    np.testing.assert_array_equal(actual["other"].to_numpy(), expected["other"].to_numpy())


def test_sparse_dataframe_mapping_rejects_unknown_column_with_ignore():
    for mod in (pd, fpd):
        with pytest.raises(KeyError, match="not found in columns"):
            mod.DataFrame({"x": [0, 1]}).astype({"absent": mod.SparseDtype("int64", 0)}, errors="ignore")


def test_sparse_dataframe_explicit_fill_points_survive_arithmetic():
    def build(mod):
        frame = mod.DataFrame({"a": [0, 2, 0, 3], "b": [0, 0, 5, 0]}, dtype="Sparse[int]")
        return frame - frame

    actual, expected = build(fpd), build(pd)
    assert_sparse_frame(actual, expected)
    assert actual.sparse.density == 3 / 8
    np.testing.assert_array_equal(actual.sparse.to_dense().to_numpy(), np.zeros((4, 2), dtype="int64"))


@pytest.mark.parametrize("kind", ["series", "frame", "mixed_frame"])
def test_dense_data_cannot_use_sparse_accessor(kind):
    def build(mod):
        if kind == "series":
            return mod.Series([0, 1])
        if kind == "frame":
            return mod.DataFrame({"a": [0, 1]})
        return mod.DataFrame({"a": mod.arrays.SparseArray([0, 1]), "b": [1, 0]})

    for mod in (pd, fpd):
        with pytest.raises(AttributeError, match="Sparse data"):
            build(mod).sparse


def test_empty_sparse_frame_density_and_accessors_match_pandas():
    for mod in (pd, fpd):
        assert np.isnan(mod.DataFrame().sparse.density)
        frame = mod.DataFrame({"a": mod.arrays.SparseArray([], dtype="float64")})
        with pytest.raises(ZeroDivisionError):
            frame.sparse.density
        assert frame.sparse.to_dense().shape == (0, 1)
        with pytest.raises(AttributeError):
            frame.sparse.npoints


@pytest.mark.parametrize("dtype", [None, "bool", "int8", "uint16", "float32", "float64"])
@pytest.mark.parametrize("dummy_na", [False, True])
@pytest.mark.parametrize("drop_first", [False, True])
def test_sparse_get_dummies_series(dtype, dummy_na, drop_first):
    options = dict(sparse=True, dtype=dtype, dummy_na=dummy_na, drop_first=drop_first)
    actual = fpd.get_dummies(fpd.Series(["b", "a", None, "b", "c"], index=[5, 4, 3, 2, 1]), **options)
    expected = pd.get_dummies(pd.Series(["b", "a", None, "b", "c"], index=[5, 4, 3, 2, 1]), **options)
    assert_sparse_frame(actual, expected)


def test_sparse_get_dummies_frame_preserves_other_columns():
    values = {"number": [3, 4, 5], "kind": ["x", None, "y"], "group": ["a", "a", "b"]}
    options = dict(sparse=True, dtype="int8", prefix={"kind": "k", "group": "g"}, dummy_na=True)
    actual = fpd.get_dummies(fpd.DataFrame(values), **options)
    expected = pd.get_dummies(pd.DataFrame(values), **options)
    assert actual.columns.tolist() == expected.columns.tolist()
    assert actual["number"].tolist() == expected["number"].tolist()
    assert str(actual["number"].dtype) == str(expected["number"].dtype)
    for name in expected.columns[1:]:
        assert_sparse_series(actual[name], expected[name])
    with pytest.raises(AttributeError):
        actual.sparse


@pytest.mark.parametrize("dtype", ["boolean", "Int64", "Float64", "UInt8"])
def test_sparse_get_dummies_rejects_extension_subtypes(dtype):
    for mod in (pd, fpd):
        with pytest.raises(TypeError):
            mod.get_dummies(["a", "b"], sparse=True, dtype=dtype)


@pytest.mark.parametrize("container", ["series", "frame"])
def test_sparse_get_dummies_string_input_needs_numpy_output_dtype(container):
    def build(mod):
        values = mod.Series(["a", "b", None], dtype="string")
        return mod.DataFrame({"kind": values}) if container == "frame" else values

    for mod in (pd, fpd):
        with pytest.raises(TypeError):
            mod.get_dummies(build(mod), sparse=True)
    assert_sparse_frame(
        fpd.get_dummies(build(fpd), sparse=True, dtype="bool"),
        pd.get_dummies(build(pd), sparse=True, dtype="bool"),
    )


@pytest.mark.parametrize("dtype", ["boolean", "Int64"])
def test_sparse_get_dummies_nullable_numeric_input_defaults_to_numpy_bool(dtype):
    actual = fpd.get_dummies(fpd.Series([True, False, None], dtype=dtype), sparse=True)
    expected = pd.get_dummies(pd.Series([True, False, None], dtype=dtype), sparse=True)
    assert_sparse_frame(actual, expected)


@pytest.mark.parametrize("columns", [None, ["k"], ["k", "k"]])
@pytest.mark.parametrize("sparse", [False, True])
def test_get_dummies_duplicate_source_columns_are_positional(columns, sparse):
    def build(mod):
        frame = mod.DataFrame([["a", "x"], ["b", "y"]], columns=["k", "k"])
        return mod.get_dummies(frame, columns=columns, sparse=sparse)

    actual, expected = build(fpd), build(pd)
    assert actual.columns.tolist() == expected.columns.tolist()
    np.testing.assert_array_equal(actual.to_numpy(), expected.to_numpy())
    if sparse:
        assert_sparse_frame(actual, expected)


@pytest.mark.parametrize("sparse", [False, True])
def test_get_dummies_duplicate_labels_keep_unencoded_numeric_column(sparse):
    def build(mod):
        frame = mod.DataFrame([["a", 1], ["b", 2]], columns=["k", "k"])
        return mod.get_dummies(frame, sparse=sparse)

    actual, expected = build(fpd), build(pd)
    assert actual.columns.tolist() == expected.columns.tolist()
    np.testing.assert_array_equal(actual.to_numpy(), expected.to_numpy())
    assert str(actual.iloc[:, 0].dtype) == str(expected.iloc[:, 0].dtype)
    if sparse:
        for position in (1, 2):
            assert_sparse_series(actual.iloc[:, position], expected.iloc[:, position])


def test_sparse_get_dummies_duplicate_columns_have_separate_prefixes():
    def build(mod):
        frame = mod.DataFrame([["a", "x"], ["b", "y"]], columns=["k", "k"])
        return mod.get_dummies(frame, sparse=True, prefix=["left", "right"])

    assert_sparse_frame(build(fpd), build(pd))


def assert_coo_matrix(actual, expected):
    assert type(actual) is type(expected)
    assert actual.shape == expected.shape
    assert actual.dtype == expected.dtype
    assert actual.nnz == expected.nnz
    np.testing.assert_array_equal(actual.data, expected.data)
    np.testing.assert_array_equal(actual.row, expected.row)
    np.testing.assert_array_equal(actual.col, expected.col)
    np.testing.assert_array_equal(actual.toarray(), expected.toarray())


def assert_series_coo(actual, expected):
    assert_coo_matrix(actual[0], expected[0])

    def label_key(label):
        if isinstance(label, tuple):
            return ("tuple", tuple(label_key(item) for item in label))
        if pd.isna(label):
            return (type(label).__name__, "<missing>")
        return (type(label).__name__, label)

    for got, want in zip(actual[1:], expected[1:], strict=True):
        # An unsorted single level is returned as singleton tuples; the
        # sorted path returns its catalog as scalar labels instead. Compare
        # nested missing labels explicitly: even pandas' index assertion
        # treats independently created (NaN,) tuples as different values.
        assert [label_key(label) for label in got] == [label_key(label) for label in want]


@pytest.mark.parametrize("dtype", ["int8", "uint16", "int64", "float32", "float64", "bool"])
@pytest.mark.parametrize("sort_labels", [False, True])
def test_sparse_series_to_coo_numeric_subtypes(dtype, sort_labels):
    def build(mod):
        index = mod.MultiIndex.from_tuples([("b", 2), ("a", 1), ("b", 1), ("a", 2)])
        values = [0, 2, 0, 3] if dtype != "bool" else [False, True, False, True]
        return mod.Series(values, index=index, dtype=mod.SparseDtype(dtype, 0))

    assert_series_coo(
        build(fpd).sparse.to_coo(sort_labels=sort_labels),
        build(pd).sparse.to_coo(sort_labels=sort_labels),
    )


@pytest.mark.parametrize("fill", [0, 7, np.nan])
@pytest.mark.parametrize("sort_labels", [False, True])
def test_sparse_series_to_coo_custom_fill_skips_missing_entries(fill, sort_labels):
    def build(mod):
        index = mod.MultiIndex.from_tuples([("b", 2), ("a", 1), ("b", 1), ("a", 2)])
        return mod.Series([0, 7, np.nan, 3], index=index, dtype=mod.SparseDtype("float32", fill))

    assert_series_coo(
        build(fpd).sparse.to_coo(sort_labels=sort_labels),
        build(pd).sparse.to_coo(sort_labels=sort_labels),
    )


@pytest.mark.parametrize(
    "rows,columns",
    [([0], [1, 2]), ([1, 0], [2]), (["two", "one"], ["three"]), ([-1], [0, 1]), ([0, 0], [1, 2])],
)
@pytest.mark.parametrize("sort_labels", [False, True])
def test_sparse_series_to_coo_partitions_actual_multiindex(rows, columns, sort_labels):
    def build(mod):
        index = mod.MultiIndex.from_tuples(
            [("b__x", 1, 2), ("a|x", 0, 1), ("b__x", 0, 1), ("a|x", 1, 2)],
            names=["one", "two", "three"],
        )
        return mod.Series([1, 0, 2, 3], index=index, dtype="Sparse[int]")

    assert_series_coo(
        build(fpd).sparse.to_coo(rows, columns, sort_labels),
        build(pd).sparse.to_coo(rows, columns, sort_labels),
    )


@pytest.mark.parametrize("empty", [False, True])
@pytest.mark.parametrize("sort_labels", [False, True])
def test_sparse_series_to_coo_declared_unused_labels(empty, sort_labels):
    def build(mod):
        index = mod.MultiIndex(
            levels=[["z", "b", "a", "unused"], [4, 2, 1]],
            codes=[[], []] if empty else [[1, 2, 1], [1, 2, 2]],
            names=["r", "c"],
        )
        return mod.Series([] if empty else [1, 2, 3], index=index, dtype="Sparse[int]")

    assert_series_coo(
        build(fpd).sparse.to_coo(sort_labels=sort_labels),
        build(pd).sparse.to_coo(sort_labels=sort_labels),
    )


def test_sparse_series_to_coo_missing_index_labels():
    def build(mod):
        index = mod.MultiIndex.from_tuples([("b", 1), (None, 2), ("a", None)])
        return mod.Series([1, 2, 3], index=index, dtype="Sparse[int]")

    assert_series_coo(build(fpd).sparse.to_coo(), build(pd).sparse.to_coo())
    for mod in (pd, fpd):
        with pytest.raises(ValueError, match="negative axis"):
            build(mod).sparse.to_coo(sort_labels=True)


@pytest.mark.parametrize(
    "rows,columns",
    [([0], [1]), ([0], [0, 1, 2]), ([4], [0, 1]), ([], [0, 1, 2]), (None, [1, 2]), ([True], [0, 2])],
)
def test_sparse_series_to_coo_invalid_level_partition(rows, columns):
    def build(mod):
        index = mod.MultiIndex.from_tuples([("a", 1, 2)], names=["one", "two", "three"])
        return mod.Series([1], index=index, dtype="Sparse[int]")

    with pytest.raises(Exception) as expected:
        build(pd).sparse.to_coo(rows, columns)
    with pytest.raises(type(expected.value)):
        build(fpd).sparse.to_coo(rows, columns)


@pytest.mark.parametrize("kind", ["flat", "one_level", "duplicate", "duplicate_names"])
def test_sparse_series_to_coo_index_validation(kind):
    for mod in (pd, fpd):
        if kind == "flat":
            index = mod.Index(["a", "b"])
        elif kind == "one_level":
            index = mod.MultiIndex.from_tuples([("a",), ("b",)])
        elif kind == "duplicate":
            index = mod.MultiIndex.from_tuples([("a", 1), ("a", 1)])
        else:
            index = mod.MultiIndex.from_tuples([("a", 1), ("b", 2)], names=["x", "x"])
        series = mod.Series([1, 2], index=index, dtype="Sparse[int]")
        with pytest.raises(ValueError):
            series.sparse.to_coo(["x"], [1]) if kind == "duplicate_names" else series.sparse.to_coo()


@pytest.mark.parametrize(
    "left,right",
    [("int8", "uint8"), ("int64", "float32"), ("bool", "float32"), ("int64", "uint64"), ("float32", "float32")],
)
def test_sparse_dataframe_to_coo_promotes_stored_subtypes(left, right):
    def build(mod):
        return mod.DataFrame({
            "a": mod.Series([0, 2, 0], dtype=mod.SparseDtype(left, 0)),
            "b": mod.Series([3, 0, 0], dtype=mod.SparseDtype(right, 0)),
        })

    assert_coo_matrix(build(fpd).sparse.to_coo(), build(pd).sparse.to_coo())


def test_sparse_dataframe_to_coo_keeps_duplicate_positions_and_missing_values():
    def build(mod):
        return mod.DataFrame(
            [[0, 1], [2, 0], [np.nan, 3]], columns=["x", "x"], index=["a", "a", "b"],
            dtype=mod.SparseDtype("float32", 0),
        )

    assert_coo_matrix(build(fpd).sparse.to_coo(), build(pd).sparse.to_coo())


@pytest.mark.parametrize("frame", [False, True])
def test_sparse_to_coo_retains_explicitly_stored_zero(frame):
    def build(mod):
        index = mod.MultiIndex.from_tuples([("b", 2), ("a", 1), ("b", 1)])
        series = mod.Series([0.1, 0, 2.7], index=index).astype("Sparse[int]")
        return series.to_frame() if frame else series

    actual, expected = build(fpd).sparse.to_coo(), build(pd).sparse.to_coo()
    assert_coo_matrix(actual, expected) if frame else assert_series_coo(actual, expected)
    matrix = actual if frame else actual[0]
    assert matrix.nnz == 2
    np.testing.assert_array_equal(matrix.data, [0, 2])


@pytest.mark.parametrize("fill", [7, np.nan, True])
def test_sparse_dataframe_to_coo_requires_zero_fill(fill):
    for mod in (pd, fpd):
        frame = mod.DataFrame({"x": mod.Series([0, 7, 3], dtype=mod.SparseDtype("float64", fill))})
        with pytest.raises(ValueError, match="fill value must be 0"):
            frame.sparse.to_coo()


def test_sparse_dataframe_to_coo_empty_shape_and_no_columns():
    def build(mod):
        return mod.DataFrame({"x": mod.Series([], dtype=mod.SparseDtype("float32", 0))})

    assert_coo_matrix(build(fpd).sparse.to_coo(), build(pd).sparse.to_coo())
    for mod in (pd, fpd):
        with pytest.raises(ValueError, match="no types given"):
            mod.DataFrame(index=[1, 2]).sparse.to_coo()


def test_sparse_get_dummies_to_scipy_coo_workflow():
    actual = fpd.get_dummies(["b", "a", "b", None], sparse=True, dummy_na=True).sparse.to_coo()
    expected = pd.get_dummies(["b", "a", "b", None], sparse=True, dummy_na=True).sparse.to_coo()
    assert_coo_matrix(actual, expected)
