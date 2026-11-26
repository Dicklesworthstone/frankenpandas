"""Index-based merges preserve user columns that resemble intermediate keys."""

import pandas as pd
import pytest

import frankenpandas as fp


@pytest.mark.parametrize("orientation", ["indexes", "left_index", "right_index"])
@pytest.mark.parametrize("collision", ["existing", "suffix", "indicator"])
def test_index_merge_keeps_all_columns(orientation, collision):
    key_name = "__fp_merge_index_key__"
    label_name = "__fp_merge_row_label__"

    def merge(module):
        left = {"business_id": [2, 1], "amount": [7, 8]}
        right = {"customer_id": [1, 2], "flag": [3, 4]}
        options = {}
        if collision == "existing":
            # Occupy the first two candidate names for both internals.
            for i, name in enumerate([key_name, key_name + "_", label_name, label_name + "_"]):
                left[name] = [20 + i, 30 + i]
                right[name] = [60 + i, 70 + i]
        elif collision == "suffix":
            for name in [key_name, label_name]:
                left[name[:-2]] = [10, 11]
                right[name[:-2]] = [12, 13]
            options["suffixes"] = ("__", "_right")
        else:
            options["indicator"] = label_name if orientation != "indexes" else key_name
        left = module.DataFrame(left, index=[2, 1])
        right = module.DataFrame(right, index=[1, 2])
        if orientation == "indexes":
            return left.merge(right, left_index=True, right_index=True, **options)
        if orientation == "left_index":
            return left.merge(right, left_index=True, right_on="customer_id", **options)
        return left.merge(right, left_on="business_id", right_index=True, **options)

    expected, actual = merge(pd), merge(fp)
    assert list(actual.columns) == list(expected.columns)
    assert list(actual.index) == list(expected.index)
    for name in expected.columns:
        assert actual[name].tolist() == expected[name].tolist()


def test_index_join_equal_indexes_keeps_real_key_column():
    def run(module):
        left = module.DataFrame({"__fp_merge_index_key__": [50, 70]}, index=[7, 5])
        right = module.DataFrame({"payload": [100, 200]}, index=[7, 5])
        return left.merge(right, left_index=True, right_index=True)

    expected, actual = run(pd), run(fp)
    assert list(actual.columns) == list(expected.columns)
    assert list(actual.index) == list(expected.index)
    assert actual["__fp_merge_index_key__"].tolist() == [50, 70]
