"""`expected_series.name` is emitted by the SHARED emitter, for every series op.

br-frankenpandas-xi5li. `series_to_expected` used to emit only `index` and `values`, and
exactly one handler (`op_series_map`) patched `name` in locally, with the note "emit it
locally here rather than globally to avoid perturbing the ~569 fixtures that omit name".
Thirty-nine other handlers carried an inline COPY of the same dict literal and so could
never pick the field up at all.

⚠️ THE 24 FIXTURES THAT PIN A NAME WERE NEVER WRONG. MEASURED, live pandas 2.2.3, on a
Series named "values":

    mode / rank / duplicated / drop_duplicates / where / mask / replace / map
        all return a result whose .name is "values"
    update is in-place and leaves it "values" too

So those fixtures pin pandas' actual answer and the oracle simply never wrote the field.
Before this change 24 of the 28 name-pinning fixtures "disagreed" with the oracle; after it,
28 agree and 0 disagree, with the packet corpus 1277/1277 green throughout.

⚠️ THE OTHER HALF OF xi5li IS DELIBERATELY NOT FIXED HERE. `compare_series_expected`
(fp-conformance lib.rs) compares index, value length and values — it does NOT read `name`.
So none of this is enforced yet on the FrankenPandas side, and turning that comparison on is
a separate change that has to answer a question no test currently asks: does FrankenPandas
propagate series names through these operations? Emitting the field is the precondition for
asking it, not the answer.
"""

from __future__ import annotations

import pytest


@pytest.fixture
def named_series():
    import pandas as pd

    return pd.Series([3, 1, 1, 2], index=[0, 1, 2, 3], name="values")


def test_shared_emitter_writes_the_name_xi5li(oracle, named_series):
    assert oracle.series_to_expected(named_series)["name"] == "values"


def test_shared_emitter_omits_an_absent_name_xi5li(oracle, named_series):
    """A nameless Series must not gain a `"name": null` key.

    ~569 fixtures omit the field entirely. Emitting an explicit null for them would be a
    corpus-wide diff for no information, and would make every one of them look like it
    asserts namelessness when it asserts nothing.
    """
    anonymous = named_series.rename(None)
    assert "name" not in oracle.series_to_expected(anonymous)


@pytest.mark.parametrize(
    "operation, extra",
    [
        ("series_rank", {}),
        ("series_duplicated", {}),
        ("series_drop_duplicates", {}),
        ("series_mode", {}),
    ],
)
def test_dispatched_ops_carry_the_name_through_xi5li(oracle, operation, extra):
    """Handlers that used to inline their own copy of the emitter's dict.

    Parameterised across four of them because the bug was per-handler duplication: fixing
    the shared emitter alone left 12 fixtures still missing the field, and only folding the
    39 inline copies back in closed it. One handler passing would not show that.
    """
    import pandas as pd

    payload = {
        "operation": operation,
        "left": {
            "name": "values",
            "index": [{"kind": "int64", "value": i} for i in range(4)],
            "values": [{"kind": "int64", "value": v} for v in (3, 1, 1, 2)],
        },
        **extra,
    }
    result = oracle.dispatch(pd, payload)
    assert result["expected_series"].get("name") == "values", (
        f"{operation} must carry the input name through — pandas does, and the fixtures "
        "pin it. A missing key here means this handler still has its own inline copy of "
        "the emitter's dict literal"
    )
