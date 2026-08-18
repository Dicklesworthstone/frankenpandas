"""The typed-datetime encoder must REFUSE the degraded state, not answer through it.

br-frankenpandas-6c6mu. `pandas_oracle._PD` is populated only inside `main()`. The typed
datetime branch of `dataframe_to_json` used to be guarded `and _PD is not None`, so a
caller that reached it without going through `main()` silently got the utf8 fallback
instead of the typed encoding it asked for.

⚠️ THAT IS NOT A HYPOTHETICAL CALLER. Sweeping the whole fixture corpus in-process —
`import pandas_oracle; pandas_oracle.dispatch(pd, req)` — is the obvious way to measure
drift and is orders of magnitude faster than one subprocess per fixture. MEASURED, one
request, two callers:

    subprocess (CLI)          {"kind": "datetime64", "value": 1705314600000000000}
    in-process, _PD unset     {"kind": "utf8", "value": "2024-01-15 10:30:00"}

which produced a published "this fixture contradicts its oracle" finding on fp_p2d_432 that
was entirely an artifact of the caller, and a wrong explanation built on top of it.

The three tests below are the three states that have to stay distinguishable: asked-for and
available, asked-for and unavailable, not asked for. Only the middle one changed.
"""

from __future__ import annotations

import pytest


@pytest.fixture
def dt_frame():
    import pandas as pd

    return pd.DataFrame({"d": pd.to_datetime(["2024-01-15 10:30:00"])})


@pytest.fixture
def unset_pd(oracle):
    """Force the degraded state, then put back whatever was there.

    Restoring the ORIGINAL value rather than assigning `pd` matters: these tests run in
    the same process as the rest of the suite, and leaving `_PD` set would mask this very
    bug for every test that ran after.
    """
    previous = oracle._PD
    oracle._PD = None
    try:
        yield oracle
    finally:
        oracle._PD = previous


def test_typed_datetime_without_pd_raises_instead_of_degrading_6c6mu(unset_pd, dt_frame):
    with pytest.raises(unset_pd.OracleError) as excinfo:
        unset_pd.dataframe_to_json(dt_frame, datetime_as_typed=True)
    message = str(excinfo.value)
    assert "_PD" in message and "main()" in message, (
        "the refusal has to name the global and where it is set, or the reader learns "
        "only that something is wrong — the whole failure mode here was a missing hint"
    )
    assert "degrades to utf8" in message, (
        "and it must say what the silent behaviour WAS, so anyone who already trusted "
        "a degraded result knows to recheck it"
    )


def test_typed_datetime_with_pd_still_emits_ticks_6c6mu(oracle, dt_frame):
    """Non-vacuity. Without this the refusal could be 'achieved' by never encoding
    typed datetimes at all, which is the same wrong answer with an extra step.
    """
    import pandas as pd

    oracle._PD = pd
    encoded = oracle.dataframe_to_json(dt_frame, datetime_as_typed=True)
    assert encoded["columns"]["d"] == [
        {"kind": "datetime64", "value": 1705314600000000000}
    ]


def test_untyped_path_is_untouched_by_the_guard_6c6mu(unset_pd, dt_frame):
    """The guard must fire ONLY when the typed encoding was requested.

    `dataframe_to_json(frame)` with no flag is the default at 97 of the 98 call sites, and
    it legitimately does not need `_PD` — breaking it would take the whole oracle down
    rather than fixing a quiet answer.
    """
    encoded = unset_pd.dataframe_to_json(dt_frame)
    assert encoded["columns"]["d"] == [
        {"kind": "utf8", "value": "2024-01-15 10:30:00"}
    ]
