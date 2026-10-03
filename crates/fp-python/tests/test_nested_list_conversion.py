"""Public API regression proposal; run against the qualified raw extension.

Each case runs in a child process so an unfixed native recursion crash cannot
kill pytest. Import failures, timeouts, signals and unexpected exception types
all fail the test; there are no optional-extension skips.
"""

import subprocess
import sys

import pytest


CONSTRUCTORS = (
    "series",
    "object_series",
    "frame_dict",
    "frame_records",
    "frame_matrix",
)

PUBLIC_CONSTRUCTORS = """
import frankenpandas as fp

def build(cell):
    if MAKER == "series":
        return fp.Series([cell])
    if MAKER == "object_series":
        return fp.Series([cell], dtype="object")
    if MAKER == "frame_dict":
        return fp.DataFrame({"a": [cell]})
    if MAKER == "frame_records":
        return fp.DataFrame([{"a": cell}])
    if MAKER == "frame_matrix":
        return fp.DataFrame([[cell]], columns=["a"])
    raise AssertionError(MAKER)

def read_cell(result):
    if MAKER in ("series", "object_series"):
        return result.tolist()[0]
    return result["a"].tolist()[0]
"""


def run_case(code: str) -> None:
    result = subprocess.run(
        [sys.executable, "-c", code],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert result.returncode == 0, (
        f"child exit {result.returncode}\n"
        f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
    )
    assert result.stdout.strip() == "PASS"


@pytest.mark.parametrize("maker", CONSTRUCTORS)
@pytest.mark.parametrize("cycle", ("self", "indirect"))
def test_cyclic_list_cells_raise_value_error(maker: str, cycle: str) -> None:
    run_case(
        f"MAKER = {maker!r}\nCYCLE = {cycle!r}\n"
        + PUBLIC_CONSTRUCTORS
        + """
cell = []
if CYCLE == "self":
    cell.append(cell)
else:
    child = [cell]
    cell.append(child)

try:
    build(cell)
except ValueError as error:
    assert str(error) == "cyclic list cells are not supported", str(error)
else:
    raise AssertionError("a cyclic list cell was accepted")

# A refusal must leave the extension usable for the next independent input.
assert fp.Series([[1, 2]]).tolist() == [[1, 2]]
print("PASS")
"""
    )


@pytest.mark.parametrize("maker", CONSTRUCTORS)
@pytest.mark.parametrize("depth", (128, 129))
def test_list_cell_nesting_is_bounded(maker: str, depth: int) -> None:
    run_case(
        f"MAKER = {maker!r}\nDEPTH = {depth}\n"
        + PUBLIC_CONSTRUCTORS
        + """
cell = 7
for _ in range(DEPTH):
    cell = [cell]

if DEPTH == 129:
    try:
        build(cell)
    except ValueError as error:
        assert str(error) == "list cell nesting exceeds 128 levels", str(error)
    else:
        raise AssertionError("excessive list-cell nesting was accepted")
else:
    restored = read_cell(build(cell))
    for _ in range(DEPTH):
        assert isinstance(restored, list) and len(restored) == 1
        restored = restored[0]
    assert restored == 7
print("PASS")
"""
    )


def test_shared_acyclic_lists_do_not_invoke_equality_or_hash() -> None:
    run_case(
        """
import frankenpandas as fp

class NoEqualityList(list):
    def __eq__(self, other):
        raise AssertionError("cycle detection invoked Python equality")

    def __hash__(self):
        raise AssertionError("cycle detection invoked Python hashing")

shared = NoEqualityList([1, None, "text", b"raw"])
cells = [[shared, shared], shared]
expected_shared = [1, None, "text", b"raw"]
expected = [[expected_shared, expected_shared], expected_shared]
assert fp.Series(cells).tolist() == expected
assert fp.Series(cells, dtype="object").tolist() == expected
assert fp.DataFrame({"a": cells})["a"].tolist() == expected
print("PASS")
"""
    )


def test_ordinary_scalar_and_host_cell_behavior_is_preserved() -> None:
    run_case(
        """
import math
import numpy as np
import frankenpandas as fp

host = object()
array = np.array([1, 2])
cells = [None, True, 7, 2.5, "text", b"raw", host, array, float("nan")]
restored = fp.Series(cells, dtype="object").tolist()
assert restored[:6] == [None, True, 7, 2.5, "text", b"raw"]
assert restored[6] is host
assert restored[7] is array
assert math.isnan(restored[8])
assert fp.Series([1, 2, 3]).tolist() == [1, 2, 3]
assert fp.Series([1, None], dtype="Int64").tolist()[1] is fp.NA
print("PASS")
"""
    )
