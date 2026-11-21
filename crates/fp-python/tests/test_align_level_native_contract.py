"""Native unsupported-case boundaries, separate from pandas parity checks."""

import pytest

import frankenpandas as fpd


def _operands():
    multi = fpd.Series(
        [1, 2],
        index=fpd.MultiIndex.from_tuples(
            [("b", 2), ("a", 1)], names=["group", "row"],
        ),
    )
    flat = fpd.Series(
        [10, 20],
        index=fpd.CategoricalIndex(["b", "c"], categories=["a", "b", "c"]),
    )
    # The constructor restores categorical metadata after extracting labels.
    assert type(flat.index).__name__ == "CategoricalIndex"
    return multi, flat


@pytest.mark.parametrize("multi_join", ["outer", "inner", "right"])
@pytest.mark.parametrize("reverse", [False, True])
def test_remapped_categorical_level_alignment_is_explicitly_unsupported(multi_join, reverse):
    multi, flat = _operands()
    first, second = (flat, multi) if reverse else (multi, flat)
    join = "left" if reverse and multi_join == "right" else multi_join
    with pytest.raises(
        NotImplementedError,
        match=r"align\(level=\.\.\.\) with remapped categorical levels",
    ):
        first.align(second, join=join, level="group")


@pytest.mark.parametrize("reverse", [False, True])
def test_categorical_flat_axis_without_level_remapping_remains_supported(reverse):
    multi, flat = _operands()
    first, second = (flat, multi) if reverse else (multi, flat)
    left, right = first.align(second, join="right" if reverse else "left", level="group")
    assert left.index.tolist() == right.index.tolist() == multi.index.tolist()
    assert list(left.index.names) == list(right.index.names) == ["group", "row"]
