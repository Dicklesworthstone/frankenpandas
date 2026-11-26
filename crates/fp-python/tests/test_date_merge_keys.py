"""Date object join keys retain their Python values and their day identity."""

from datetime import date

import pandas as pd
import pytest

import frankenpandas as fp


@pytest.mark.parametrize("how", ["inner", "left", "right", "outer"])
@pytest.mark.parametrize("sort", [False, True])
def test_merge_date_keys_matches_pandas(how, sort):
    early, late = date(1920, 2, 29), date(2024, 2, 29)
    left = {"k": ["label", late, None, early, late], "l": [1, 2, 3, 4, 5]}
    right = {"k": [late, late, "label", None, date(2030, 1, 1)], "r": [6, 7, 8, 9, 10]}

    def result(module):
        return module.DataFrame(left).merge(module.DataFrame(right), on="k", how=how, sort=sort)

    expected, actual = result(pd), result(fp)
    assert list(actual.columns) == list(expected.columns)
    for name in expected.columns:
        assert str(actual[name].dtype) == str(expected[name].dtype)
        for found, wanted in zip(actual[name].tolist(), expected[name].tolist(), strict=True):
            if pd.isna(wanted):
                assert pd.isna(found)
            else:
                assert found == wanted
                if isinstance(wanted, date):
                    assert type(found) is date


def test_sorted_merge_refuses_incomparable_date_and_int():
    for module in [pd, fp]:
        frame = module.DataFrame({"k": [date(1970, 1, 1), 0]})
        with pytest.raises(TypeError):
            frame.merge(frame, on="k", sort=True)


def test_date_subclass_with_custom_identity_remains_opaque():
    class SameDay(date):
        def __eq__(self, other):
            return isinstance(other, SameDay)

        def __hash__(self):
            return 1

    left = fp.DataFrame({"k": [SameDay(2000, 1, 1)]})
    right = fp.DataFrame({"k": [SameDay(2020, 4, 30)]})
    with pytest.raises(ValueError, match="opaque host object join keys"):
        left.merge(right, on="k")


@pytest.mark.parametrize(
    ("left", "right", "how", "number_of_keys"),
    [
        ([], [date(2000, 1, 1), 1], "inner", 1),
        ([date(2000, 1, 1), 1], [7], "inner", 1),
        ([date(2000, 1, 1)], [0], "left", 1),
        ([0], [date(2000, 1, 1)], "outer", 1),
        (["text"], [date(2000, 1, 1)], "outer", 1),
        ([date(2000, 1, 1)], [0], "inner", 2),
    ],
)
def test_sorted_merge_date_edges(left, right, how, number_of_keys):
    def result(module):
        def frame(keys):
            return module.DataFrame({"k": module.Series(keys, dtype="object"), "extra": [1] * len(keys)})

        return frame(left).merge(frame(right), on=["k", "extra"][:number_of_keys], how=how, sort=True)

    try:
        expected = result(pd)
    except TypeError:
        with pytest.raises(TypeError):
            result(fp)
    else:
        assert result(fp)["k"].tolist() == expected["k"].tolist()


def test_date_keys_match_in_composite_and_grouped_asof_joins():
    first, second = date(1900, 1, 1), date(2030, 5, 10)

    def result(module):
        left = module.DataFrame({"day": [first, second, first], "time": [1, 2, 3]})
        right = module.DataFrame({"day": [first, second, first], "time": [1, 1, 3], "r": [10, 20, 30]})
        exact = left.merge(right, on=["day", "time"], validate="1:1")
        asof = module.merge_asof(left, right, on="time", by="day")
        return exact["day"].tolist(), exact["r"].tolist(), asof["r"].tolist()

    assert result(fp) == result(pd)
