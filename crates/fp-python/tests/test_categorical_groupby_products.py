"""Categorical multi-key reductions against the pandas 2.2 oracle (fvsao.40)."""

from __future__ import annotations

import numpy as np
import pandas as pd
import pytest

import frankenpandas as fpd


def _missing(value):
    if value is None or value is fpd.NA or value is fpd.NaT:
        return True
    try:
        return bool(pd.isna(value))
    except (TypeError, ValueError):
        return False


def _cell(actual, expected):
    if isinstance(expected, tuple):
        assert isinstance(actual, tuple)
        assert len(actual) == len(expected)
        for a, e in zip(actual, expected):
            _cell(a, e)
    elif _missing(expected):
        assert _missing(actual), (actual, expected)
    elif isinstance(expected, pd.Interval):
        _cell(actual.left, expected.left)
        _cell(actual.right, expected.right)
        assert actual.closed == expected.closed
    elif isinstance(expected, (pd.Timestamp, pd.Timedelta)):
        assert actual.value == expected.value
        if isinstance(expected, pd.Timestamp):
            assert str(actual.tz) == str(expected.tz)
    elif isinstance(expected, (bool, np.bool_)):
        assert isinstance(actual, (bool, np.bool_))
        assert bool(actual) == bool(expected)
    elif isinstance(expected, (int, np.integer)):
        # Do not float-cast: masked integer results can exceed 2**53.
        assert actual == int(expected)
    elif isinstance(expected, (float, np.floating)):
        assert actual == pytest.approx(float(expected), rel=1e-12, abs=1e-12)
    else:
        assert actual == expected


def _values(actual, expected):
    actual, expected = list(actual), list(expected)
    assert len(actual) == len(expected)
    for a, e in zip(actual, expected):
        _cell(a, e)


def _dtype(actual, expected):
    assert str(actual) == str(expected)
    if isinstance(expected, pd.CategoricalDtype):
        _values(actual.categories, expected.categories)
        assert actual.ordered == expected.ordered


def _index(actual, expected):
    _values(actual, expected)
    assert list(actual.names) == list(expected.names)
    assert actual.nlevels == expected.nlevels
    if isinstance(expected, pd.MultiIndex):
        assert len(actual.levels) == len(expected.levels)
        for a, e in zip(actual.levels, expected.levels):
            _values(a, e)
            _dtype(a.dtype, e.dtype)
        for a, e in zip(actual.codes, expected.codes):
            _values(a, e)
    else:
        _dtype(actual.dtype, expected.dtype)


def _result(actual, expected):
    assert actual.shape == expected.shape
    _index(actual.index, expected.index)
    if isinstance(expected, pd.DataFrame):
        assert isinstance(actual, fpd.DataFrame)
        _index(actual.columns, expected.columns)
        for position in range(len(expected.columns)):
            a, e = actual.iloc[:, position], expected.iloc[:, position]
            _dtype(a.dtype, e.dtype)
            _values(a.tolist(), e.tolist())
    else:
        assert isinstance(actual, fpd.Series)
        assert actual.name == expected.name
        _dtype(actual.dtype, expected.dtype)
        _values(actual.tolist(), expected.tolist())


def _frame(module, second_categorical=False):
    second = [2, 1, 1, 2, 2, 1]
    if second_categorical:
        second = module.Categorical(
            ["y", "x", "x", "y", "y", "x"],
            categories=["unused", "y", "x"],
            ordered=False,
        )
    return module.DataFrame(
        {
            "a": module.Categorical(
                ["b", "a", "b", None, "b", "a"],
                categories=["c", "b", "a"],
                ordered=True,
            ),
            "b": second,
            "v": [1, 2, 3, 4, 5, 6],
            "w": [1.0, None, 3.0, 4.0, 5.0, 6.0],
        }
    )


def _mixed_key_frame(module):
    return module.DataFrame(
        {
            "a": module.Categorical(
                ["a/b", "a", None, "a/b", "a"], categories=["unused", "a/b", "a"]
            ),
            "b": module.Series([1, "1", float("nan"), None, "b/1"], dtype=object),
            "v": [1, 2, 3, 4, 5],
        }
    )



@pytest.mark.parametrize("sort", [True, False])
@pytest.mark.parametrize("observed", [True, False])
@pytest.mark.parametrize("ascending", [True, False])
@pytest.mark.parametrize("selected", [True, False])
@pytest.mark.parametrize("keys", ["a", ["a", "b"]])
def test_group_numbers_use_observed_ranks(sort, observed, ascending, selected, keys):
    def run(module):
        frame = _frame(module)
        frame.index = module.Index([8, 5, 8, 2, 0, 4], name="source")
        groups = frame.groupby(keys, observed=observed, sort=sort)
        return (groups["v"] if selected else groups).ngroup(ascending=ascending)

    _result(run(fpd), run(pd))


@pytest.mark.parametrize("dropped", [[], [None, None]])
@pytest.mark.parametrize("ascending", [True, False])
def test_group_numbers_with_no_observed_rows(dropped, ascending):
    def run(module):
        frame = module.DataFrame({
            "k": module.Categorical(dropped, categories=["unused", "more"]),
            "v": module.Series([1] * len(dropped), dtype="int64"),
        })
        return frame.groupby("k", observed=False)["v"].ngroup(ascending=ascending)

    _result(run(fpd), run(pd))


@pytest.mark.parametrize("sort", [True, False])
@pytest.mark.parametrize("dropna", [True, False])
@pytest.mark.parametrize("keys", ["a", ["a", "b"]])
def test_frame_category_group_numbers_with_missing_keys(sort, dropna, keys):
    def run(module):
        return _frame(module).groupby(keys, observed=False, sort=sort, dropna=dropna).ngroup(False)

    _result(run(fpd), run(pd))


@pytest.mark.parametrize("sort", [True, False])
@pytest.mark.parametrize("observed", [True, False])
@pytest.mark.parametrize("as_index", [True, False])
@pytest.mark.parametrize("keys", ["a", ["a", "b"]])
def test_group_unique_keeps_empty_categories(sort, observed, as_index, keys):
    def run(module):
        frame = _frame(module)
        return frame.groupby(keys, observed=observed, sort=sort, as_index=as_index)["v"].unique()

    actual, expected = run(fpd), run(pd)
    _index(actual.index, expected.index)
    if not as_index:
        _index(actual.columns, expected.columns)
        for key in [keys] if isinstance(keys, str) else keys:
            _values(actual[key].tolist(), expected[key].tolist())
            _dtype(actual[key].dtype, expected[key].dtype)
        actual, expected = actual["v"], expected["v"]
    assert actual.name == expected.name
    assert str(actual.dtype) == str(expected.dtype) == "object"
    for a, e in zip(actual.tolist(), expected.tolist(), strict=True):
        if isinstance(e, np.ndarray):
            assert isinstance(a, np.ndarray)
            assert a.dtype == e.dtype
            np.testing.assert_array_equal(a, e)
        else:
            assert np.isnan(e)
            assert np.isnan(a)


REDUCTIONS = [
    "sum", "prod", "count", "size", "nunique", "min", "max", "mean",
    "median", "first", "last", "std", "var", "sem", "any", "all",
]


@pytest.mark.parametrize("operation", REDUCTIONS)
@pytest.mark.parametrize("sort", [True, False])
@pytest.mark.parametrize("dropna", [True, False])
@pytest.mark.parametrize("as_index", [True, False])
@pytest.mark.parametrize("selected", [True, False])
@pytest.mark.parametrize("second_categorical", [True, False])
def test_categorical_product_reductions(
    operation, sort, dropna, as_index, selected, second_categorical
):
    def run(module):
        grouped = _frame(module, second_categorical).groupby(
            ["a", "b"], observed=False, sort=sort, dropna=dropna, as_index=as_index
        )
        if selected:
            grouped = grouped["v"]
        return getattr(grouped, operation)()

    # pandas 2.2 DataFrameGroupBy.nunique(as_index=False) raises when
    # expanding unobserved combinations; the selected Series path works.
    if operation == "nunique" and not selected and not as_index:
        with pytest.raises(ValueError) as expected:
            run(pd)
        with pytest.raises(ValueError) as actual:
            run(fpd)
        assert str(actual.value) == str(expected.value)
    else:
        _result(run(fpd), run(pd))


@pytest.mark.parametrize("sort", [True, False])
@pytest.mark.parametrize("dropna", [True, False])
@pytest.mark.parametrize("second_categorical", [True, False])
@pytest.mark.parametrize("selected", [True, False])
def test_observed_true_still_returns_only_observed_groups(
    sort, dropna, second_categorical, selected
):
    def run(module):
        grouped = _frame(module, second_categorical).groupby(
            ["a", "b"], observed=True, sort=sort, dropna=dropna
        )
        return (grouped["v"] if selected else grouped).sum()

    _result(run(fpd), run(pd))


@pytest.mark.parametrize("operation", ["sum", "prod", "min", "max", "first", "last"])
@pytest.mark.parametrize("dtype", ["Int64", "Float64", "boolean", "bool", "float32"])
@pytest.mark.parametrize("selected", [True, False])
def test_product_fill_preserves_reduction_dtypes(operation, dtype, selected):
    def run(module):
        values = [True, False, True] if dtype in {"boolean", "bool"} else [2, 4, 8]
        frame = module.DataFrame(
            {
                "a": module.Categorical(["b", "a", "b"], categories=["c", "b", "a"]),
                "b": [2, 1, 1],
                "v": module.Series(values, dtype=dtype),
            }
        )
        grouped = frame.groupby(["a", "b"], observed=False)
        return getattr(grouped["v"] if selected else grouped, operation)()

    _result(run(fpd), run(pd))


@pytest.mark.parametrize("operation", ["min", "max", "first", "last", "sum"])
def test_masked_integer_gaps_do_not_round_observed_values(operation):
    def run(module):
        frame = module.DataFrame(
            {
                "a": module.Categorical(["b", "a"], categories=["c", "b", "a"]),
                "b": [2, 1],
                "v": module.Series([2**60 + 3, 2**60 + 7], dtype="Int64"),
            }
        )
        return getattr(frame.groupby(["a", "b"], observed=False)["v"], operation)()

    _result(run(fpd), run(pd))


@pytest.mark.parametrize("min_count", [0, 1, 2, 3])
@pytest.mark.parametrize("selected", [True, False])
def test_sum_empty_combinations_are_zero_even_with_min_count(min_count, selected):
    def run(module):
        frame = module.DataFrame(
            {
                "a": module.Categorical(["b", "b", "a"], categories=["c", "b", "a"]),
                "b": [2, 2, 1],
                "v": module.Series([None, None, 7], dtype="Int64"),
            }
        )
        grouped = frame.groupby(["a", "b"], observed=False)
        return (grouped["v"] if selected else grouped).sum(min_count=min_count)

    _result(run(fpd), run(pd))


@pytest.mark.parametrize("ordinary", [False, True])
@pytest.mark.parametrize("as_index", [False, True])
@pytest.mark.parametrize("operation", ["sum", "prod", "count", "size", "min", "mean"])
def test_empty_inputs_keep_categories_and_empty_level_catalogs(ordinary, as_index, operation):
    def run(module):
        # An empty ordinary object key makes a zero-row product; two
        # categorical keys still produce the complete six-row product.
        second = module.Series([], dtype=object) if ordinary else module.Categorical(
            [], categories=["y", "x"]
        )
        frame = module.DataFrame(
            {
                "a": module.Categorical([], categories=["c", "b", "a"], ordered=True),
                "b": second,
                "v": module.Series([], dtype="int64"),
            }
        )
        return getattr(
            frame.groupby(["a", "b"], observed=False, as_index=as_index), operation
        )()

    _result(run(fpd), run(pd))



@pytest.mark.parametrize("sort", [False, True])
def test_empty_category_domain_keeps_other_nonempty_level_catalogs(sort):
    def run(module):
        frame = module.DataFrame(
            {
                "a": module.Categorical([None, None], categories=[]),
                "b": ["z", "a"],
                "v": [1, 2],
            }
        )
        return frame.groupby(["a", "b"], observed=False, sort=sort).sum()

    _result(run(fpd), run(pd))

@pytest.mark.parametrize("sort", [False, True])
@pytest.mark.parametrize("dropna", [False, True])
def test_missing_and_mixed_type_keys_remain_distinct(sort, dropna):
    def run(module):
        return _mixed_key_frame(module).groupby(
            ["a", "b"], observed=False, sort=sort, dropna=dropna
        )["v"].sum()

    actual = run(fpd)
    if not sort:
        _result(actual, run(pd))
        return

    # DISC-038: pandas 2.2.3 crashes while sorting this expanded product.
    # Preserve typed keys and the complete product: categories in declared
    # order, then numeric/string/missing ordinary keys. This is an explicit
    # integrity contract, not a successful comparison with pandas.groupby.
    categories = ["unused", "a/b", "a"]
    missing = [] if dropna else [None]
    index = pd.MultiIndex.from_product(
        [
            pd.CategoricalIndex(
                categories + missing, categories=categories, ordered=False, name="a"
            ),
            pd.Index([1, "1", "b/1"] + missing, dtype=object, name="b"),
        ]
    )
    values = (
        [0, 0, 0, 1, 0, 0, 0, 2, 5]
        if dropna
        else [0, 0, 0, 0, 1, 0, 0, 4, 0, 2, 5, 0, 0, 0, 0, 3]
    )
    _result(actual, pd.Series(values, index=index, dtype="int64", name="v"))


@pytest.mark.parametrize("dropna", [False, True])
def test_pandas_223_sorted_mixed_product_bug_witness_disc038(dropna):
    assert pd.__version__ == "2.2.3", "Review DISC-038 when the pinned pandas oracle changes"
    frame = _mixed_key_frame(pd)
    # The observed reduction handles the typed keys correctly; only the
    # unobserved product expansion raises the cross-type sorting exception.
    observed = frame.groupby(
        ["a", "b"], observed=True, sort=True, dropna=dropna
    )["v"].sum()
    expected_keys = (
        [("a/b", 1), ("a", "1"), ("a", "b/1")]
        if dropna
        else [("a/b", 1), ("a/b", None), ("a", "1"), ("a", "b/1"), (None, None)]
    )
    _values(observed.index, expected_keys)
    assert observed.tolist() == ([1, 2, 5] if dropna else [1, 4, 2, 5, 3])
    assert str(observed.dtype) == "int64"
    with pytest.raises(TypeError, match="^'<' not supported between instances of 'str' and 'int'$"):
        frame.groupby(["a", "b"], observed=False, sort=True, dropna=dropna)["v"].sum()


@pytest.mark.parametrize("dtype", [object, "string"])
def test_string_sum_matches_empty_combination_behavior(dtype):
    def run(module):
        frame = module.DataFrame(
            {
                "a": module.Categorical(["b", "a"], categories=["c", "b", "a"]),
                "b": [2, 1],
                "v": module.Series(["red", "blue"], dtype=dtype),
            }
        )
        return frame.groupby(["a", "b"], observed=False)["v"].sum()

    if dtype == "string":
        with pytest.raises(ValueError) as expected:
            run(pd)
        with pytest.raises(ValueError) as actual:
            run(fpd)
        assert str(actual.value) == str(expected.value)
    else:
        _result(run(fpd), run(pd))


@pytest.mark.parametrize("operation", ["min", "first", "sum"])
def test_timedelta_results_fill_with_nat_or_object_zero(operation):
    def run(module):
        frame = module.DataFrame(
            {
                "a": module.Categorical(["b", "a"], categories=["c", "b", "a"]),
                "b": [2, 1],
                "v": module.to_timedelta([2, 3], unit="D"),
            }
        )
        return getattr(frame.groupby(["a", "b"], observed=False)["v"], operation)()

    _result(run(fpd), run(pd))


@pytest.mark.parametrize("spec", ["list", "dict", "named", "selected"])
def test_composed_aggregations_use_each_reductions_empty_fill(spec):
    def run(module):
        grouped = _frame(module).groupby(["a", "b"], observed=False, sort=False)
        if spec == "list":
            return grouped.agg(["sum", "prod", "mean"])
        if spec == "dict":
            return grouped.agg({"w": "max", "v": ["sum", "mean"]})
        if spec == "named":
            return grouped.agg(total=("v", "sum"), average=("w", "mean"))
        return grouped["v"].agg(["sum", "prod", "mean"])

    _result(run(fpd), run(pd))


@pytest.mark.parametrize("sort", [False, True])
@pytest.mark.parametrize("dropna", [False, True])
def test_row_operations_and_group_iteration_do_not_expand_the_product(sort, dropna):
    actual = _frame(fpd).groupby(["a", "b"], observed=False, sort=sort, dropna=dropna)
    expected = _frame(pd).groupby(["a", "b"], observed=False, sort=sort, dropna=dropna)
    assert actual.ngroups == expected.ngroups
    _result(actual.head(1), expected.head(1))
    if dropna:
        expected_transform = expected["v"].transform("sum")
    else:
        # DISC-039: observed=False must not erase the real (missing, 2)
        # group's value. Pin values, dtype, name, and the original row axis.
        expected_transform = pd.Series([6, 8, 3, 4, 6, 8], dtype="int64", name="v")
    _result(actual["v"].transform("sum"), expected_transform)
    _result(actual.ngroup(), expected.ngroup())
    actual_groups, expected_groups = list(actual), list(expected)
    assert len(actual_groups) == len(expected_groups)
    for (ak, av), (ek, ev) in zip(actual_groups, expected_groups):
        _cell(ak, ek)
        _result(av, ev)


@pytest.mark.parametrize("sort", [False, True])
def test_pandas_223_transform_drops_null_group_bug_witness_disc039(sort):
    assert pd.__version__ == "2.2.3", "Review DISC-039 when the pinned pandas oracle changes"
    frame = _frame(pd)
    grouped = frame.groupby(["a", "b"], observed=False, sort=sort, dropna=False)
    assert grouped.ngroups == 4
    assert grouped.ngroup().tolist() == ([1, 2, 0, 3, 1, 2] if sort else [0, 1, 2, 3, 0, 1])
    missing_groups = [(key, part) for key, part in grouped if _missing(key[0])]
    assert len(missing_groups) == 1
    key, part = missing_groups[0]
    assert key[1] == 2
    assert part.index.tolist() == [3]
    assert part["v"].tolist() == [4]
    # Negative control: the same grouping reports the missing-key group,
    # then drops its value and promotes the transform result to float64.
    pd.testing.assert_series_equal(
        grouped["v"].transform("sum"),
        pd.Series([6.0, 8.0, 3.0, np.nan, 6.0, 8.0], dtype="float64", name="v"),
    )
    # observed=True does not expand empty products and retains every row.
    pd.testing.assert_series_equal(
        frame.groupby(["a", "b"], observed=True, sort=sort, dropna=False)["v"].transform("sum"),
        pd.Series([6, 8, 3, 4, 6, 8], dtype="int64", name="v"),
    )


@pytest.mark.parametrize("operation", ["idxmin", "idxmax"])
@pytest.mark.parametrize("selected", [False, True])
def test_extreme_index_reductions_raise_for_unobserved_empty_groups(operation, selected):
    def run(module):
        grouped = _frame(module).groupby(["a", "b"], observed=False)
        return getattr(grouped["v"] if selected else grouped, operation)()

    with pytest.raises(ValueError) as expected:
        run(pd)
    with pytest.raises(ValueError) as actual:
        run(fpd)
    assert str(actual.value) == str(expected.value)


def test_unmodelled_results_refuse_instead_of_dropping_combinations():
    grouped = _frame(fpd).groupby(["a", "b"], observed=False)
    with pytest.raises(NotImplementedError):
        grouped["v"].value_counts()
    with pytest.raises(NotImplementedError):
        grouped["v"].agg(lambda values: values.sum())
    with pytest.raises(NotImplementedError):
        grouped.quantile()


def test_complete_product_does_not_trigger_unobserved_only_guards():
    def grouped(module):
        frame = module.DataFrame(
            {
                "a": module.Categorical(["b", "b", "a", "a"], categories=["b", "a"]),
                "b": [2, 1, 2, 1],
                "v": [4, 3, 2, 1],
            }
        )
        return frame.groupby(["a", "b"], observed=False)

    actual, expected = grouped(fpd), grouped(pd)
    _result(actual.idxmax(), expected.idxmax())
    _result(actual.ngroup(), expected.ngroup())


def test_cartesian_overflow_is_lazy_and_reports_memory_error():
    # Sixty-five binary key domains cannot fit in usize. Construction,
    # iteration and per-input-row operations must not allocate that product.
    keys = [f"k{i}" for i in range(65)]
    data = {key: fpd.Categorical(["a"], categories=["a", "b"]) for key in keys}
    data["v"] = [1]
    grouped = fpd.DataFrame(data).groupby(keys, observed=False)
    assert grouped.ngroups == 1
    assert grouped.head(1)["v"].tolist() == [1]
    assert grouped.ngroup().tolist() == [0]
    with pytest.raises(MemoryError, match="categorical groupby result is too large"):
        grouped.size()
