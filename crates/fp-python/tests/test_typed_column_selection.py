"""Typed column labels must not be selected by their storage text.

Regression coverage for br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.32:
the scalar [] path rejected '0' for an integer column 0, while list reads,
subset operations, and several other entry points silently selected it.
"""

from __future__ import annotations

import numpy as np
import pytest

pd = pytest.importorskip("pandas")
fpd = pytest.importorskip("frankenpandas")


def _frame(module, label=0):
    return module.DataFrame(
        [[3, 10, 1], [1, 20, 1], [2, 30, 2]],
        columns=[label, "value", "other"],
        index=["a", "b", "c"],
    )


def _assert_same(got, expected):
    assert type(got).__name__ == type(expected).__name__
    assert got.index.tolist() == expected.index.tolist()
    assert list(got.index.names) == list(expected.index.names)
    if isinstance(expected, pd.DataFrame):
        assert got.columns.tolist() == expected.columns.tolist()
        assert [type(v) for v in got.columns.tolist()] == [
            type(v) for v in expected.columns.tolist()
        ]
        assert list(got.columns.names) == list(expected.columns.names)
        assert got.shape == expected.shape
        for position in range(expected.shape[1]):
            actual = got.iloc[:, position]
            reference = expected.iloc[:, position]
            assert str(actual.dtype) == str(reference.dtype)
            np.testing.assert_array_equal(actual.to_numpy(), reference.to_numpy())
    else:
        assert got.name == expected.name
        assert str(got.dtype) == str(expected.dtype)
        np.testing.assert_array_equal(got.to_numpy(), expected.to_numpy())


_REQUIRED_KEYS = {
    "scalar read": lambda m, frame, key: frame[key],
    "list read": lambda m, frame, key: frame[[key]],
    "mixed list read": lambda m, frame, key: frame[["value", key]],
    "numpy unicode read": lambda m, frame, key: frame[np.array([key])],
    "numpy object read": lambda m, frame, key: frame[np.array([key], dtype=object)],
    "Index read": lambda m, frame, key: frame[m.Index([key])],
    "Series read": lambda m, frame, key: frame[m.Series([key])],
    "loc scalar": lambda m, frame, key: frame.loc[:, key],
    "loc list": lambda m, frame, key: frame.loc[:, [key]],
    "loc tuple": lambda m, frame, key: frame.loc[:, (key,)],
    "loc numpy": lambda m, frame, key: frame.loc[:, np.array([key])],
    "loc Index": lambda m, frame, key: frame.loc[:, m.Index([key])],
    "loc Series": lambda m, frame, key: frame.loc[:, m.Series([key])],
    "at": lambda m, frame, key: frame.at["a", key],
    "xs": lambda m, frame, key: frame.xs(key, axis=1),
    "dropna": lambda m, frame, key: frame.dropna(subset=[key]),
    "dropna tuple": lambda m, frame, key: frame.dropna(subset=(key,)),
    "duplicated": lambda m, frame, key: frame.duplicated(subset=key),
    "drop_duplicates": lambda m, frame, key: frame.drop_duplicates(subset=[key]),
    "set_index": lambda m, frame, key: frame.set_index(key),
    "sort scalar": lambda m, frame, key: frame.sort_values(key),
    "sort list": lambda m, frame, key: frame.sort_values([key]),
    "nlargest": lambda m, frame, key: frame.nlargest(2, key),
    "nsmallest": lambda m, frame, key: frame.nsmallest(2, [key]),
    "merge on": lambda m, frame, key: m.merge(frame, frame, on=key),
    "merge left_on": lambda m, frame, key: m.merge(frame, frame, left_on=key, right_on="other"),
    "merge right_on": lambda m, frame, key: m.merge(frame, frame, left_on="other", right_on=key),
    "pivot_table index": lambda m, frame, key: frame.pivot_table(index=key, values="value", aggfunc="sum"),
    "pivot_table columns": lambda m, frame, key: frame.pivot_table(index="other", columns=key, values="value", aggfunc="sum"),
    "pivot_table values": lambda m, frame, key: frame.pivot_table(index="other", values=key, aggfunc="sum"),
    "get_dummies": lambda m, frame, key: m.get_dummies(frame, columns=[key]),
    "join": lambda m, frame, key: frame.join(m.DataFrame({"joined": [100, 200, 300]}, index=[1, 2, 3]), on=key),
}


@pytest.mark.parametrize("label", [0, 1.5, True])
@pytest.mark.parametrize("operation", list(_REQUIRED_KEYS))
def test_string_does_not_alias_typed_column(label, operation):
    key = str(label)
    for module in (pd, fpd):
        frame = _frame(module, label)
        with pytest.raises(KeyError):
            _REQUIRED_KEYS[operation](module, frame, key)
        # Failed reads and subset operations leave the frame intact.
        _assert_same(frame, _frame(pd, label))


@pytest.mark.parametrize("key_kind", ["scalar", "list", "mixed", "numpy", "Index", "Series"])
def test_get_returns_the_default_for_a_text_alias(key_kind):
    for module in (pd, fpd):
        keys = {
            "scalar": "0",
            "list": ["0"],
            "mixed": [0, "0"],
            "numpy": np.array(["0"], dtype=object),
            "Index": module.Index(["0"]),
            "Series": module.Series(["0"]),
        }
        sentinel = object()
        assert _frame(module).get(keys[key_kind], sentinel) is sentinel
        assert _frame(module).get(keys[key_kind]) is None


@pytest.mark.parametrize("items", [["0"], ["0", "value"], [0, "value"], ["value", 0], ["absent", 0]])
def test_filter_ignores_only_labels_that_are_absent(items):
    _assert_same(_frame(fpd).filter(items=items), _frame(pd).filter(items=items))


_VALID_KEYS = {
    "list read": lambda frame: frame[[0, "value"]],
    "loc scalar": lambda frame: frame.loc[:, 0],
    "loc list": lambda frame: frame.loc[:, [0, "value"]],
    "xs": lambda frame: frame.xs(0, axis=1),
    "get scalar": lambda frame: frame.get(0),
    "get list": lambda frame: frame.get([0, "value"]),
    "dropna": lambda frame: frame.dropna(subset=[0]),
    "duplicated": lambda frame: frame.duplicated(subset=[0]),
    "drop_duplicates": lambda frame: frame.drop_duplicates(subset=[0]),
    "set_index": lambda frame: frame.set_index(0),
    "sort": lambda frame: frame.sort_values([0]),
    "nlargest": lambda frame: frame.nlargest(2, 0),
    "nsmallest": lambda frame: frame.nsmallest(2, [0]),
}


@pytest.mark.parametrize("operation", list(_VALID_KEYS))
def test_existing_typed_keys_still_select_their_columns(operation):
    _assert_same(_VALID_KEYS[operation](_frame(fpd)), _VALID_KEYS[operation](_frame(pd)))


@pytest.mark.parametrize("key", ["0", ["0"], ["value", "0"]])
def test_string_labels_and_real_duplicates_keep_their_values(key):
    def frame(module):
        return module.DataFrame([[1, 9, 3], [2, 8, 4]], columns=["0", "0", "value"])

    _assert_same(frame(fpd)[key], frame(pd)[key])
    _assert_same(frame(fpd).get(key), frame(pd).get(key))


@pytest.mark.parametrize("key", ["a", ("a", "x"), [("b", "x"), ("a", "x")]])
def test_multiindex_column_selection_and_get_are_preserved(key):
    def frame(module):
        columns = module.MultiIndex.from_tuples([("a", "x"), ("a", "y"), ("b", "x")])
        return module.DataFrame([[1, 2, 3], [4, 5, 6]], columns=columns)

    _assert_same(frame(fpd)[key], frame(pd)[key])
    _assert_same(frame(fpd).get(key), frame(pd).get(key))


def test_row_boolean_masks_do_not_become_column_label_checks():
    for mask in ([True, False, True], np.array([True, False, True])):
        _assert_same(_frame(fpd)[mask], _frame(pd)[mask])
        _assert_same(_frame(fpd).get(mask), _frame(pd).get(mask))
    masks = [module.Series([True, False, True], index=["a", "b", "c"]) for module in (fpd, pd)]
    _assert_same(_frame(fpd)[masks[0]], _frame(pd)[masks[1]])
    _assert_same(_frame(fpd).get(masks[0]), _frame(pd).get(masks[1]))


@pytest.mark.parametrize("operation", ["setitem", "scalar setitem", "assign", "loc", "at"])
def test_text_alias_writes_never_overwrite_the_typed_column(operation):
    # Representing both 0 and '0' in one frame remains e672h's structural
    # task. Until supported, these writes must fail without corrupting 0.
    frame = _frame(fpd)
    with pytest.raises((KeyError, NotImplementedError)):
        if operation == "setitem":
            frame[["0"]] = [[100], [200], [300]]
        elif operation == "scalar setitem":
            frame["0"] = 100
        elif operation == "assign":
            frame.assign(**{"0": 100})
        elif operation == "loc":
            frame.loc[:, "0"] = 100
        else:
            frame.at["a", "0"] = 100
    _assert_same(frame, _frame(pd))


@pytest.mark.parametrize("operation", ["setitem", "loc", "at", "new string"])
def test_writes_to_typed_or_new_string_columns_still_work(operation):
    results = []
    for module in (fpd, pd):
        frame = _frame(module)
        if operation == "setitem":
            frame[[0]] = [[100], [200], [300]]
        elif operation == "loc":
            frame.loc[:, 0] = 100
        elif operation == "at":
            frame.at["a", 0] = 100
        else:
            frame.loc[:, "new"] = 100
        results.append(frame)
    _assert_same(*results)


@pytest.mark.parametrize("key", [0, [0]])
@pytest.mark.parametrize("how", ["left", "inner"])
def test_join_keeps_typed_keys_when_forwarding_to_merge(key, how):
    results = []
    for module in (fpd, pd):
        right = module.DataFrame({"joined": [100, 200, 300]}, index=[1, 2, 3])
        results.append(_frame(module).join(right, on=key, how=how))
    _assert_same(*results)


@pytest.mark.parametrize(
    "labels",
    [
        [("key", ""), ("value", "open"), ("value", "close")],
        [("value", "open"), ("value", "open"), ("value", "close")],
        [(0, "open"), (1, "close"), (0, "high")],
        [("prices", "value", "open"), ("prices", "value", "close"), ("group", "key", "")],
    ],
)
@pytest.mark.parametrize("rows", [slice(None), [2, 0], [True, False, True]])
def test_iloc_series_name_is_the_full_multiindex_column_label(labels, rows):
    frames = [
        module.DataFrame(
            [[1, 10, 100], [2, 20, 200], [3, 30, 300]],
            columns=module.MultiIndex.from_tuples(labels),
            index=["a", "b", "c"],
        )
        for module in (fpd, pd)
    ]
    for position in (0, 1, -1):
        _assert_same(frames[0].iloc[rows, position], frames[1].iloc[rows, position])
