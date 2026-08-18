"""The adapter's own argument-validation contract, asserted where it belongs.

br-frankenpandas-nvnvr, bucket 4. Six packet fixtures pin messages like
"dataframe_from_series requires at least one series payload" as an expected ERROR.
Those are not parity assertions: pandas is never invoked, because there is nothing
to pass it. They are HARNESS-CONTRACT tests wearing fixture clothing, and the
conformance corpus cannot attest them — `error_origin` is `oracle_adapter` for all
six, which is exactly why they sit in the freshness residue.

⚠️ MEASURED BEFORE WRITING THIS: **0 of the 6 were covered anywhere else.** So the
fixtures are the ONLY thing currently asserting this contract, and retiring them
without a replacement would LOSE coverage rather than remove redundancy. My first
description of them as "reclassify or retire" was too casual. These tests are the
replacement; only once they exist is the disposal of those fixtures a safe
bookkeeping decision rather than a silent loss.

Each test asserts BOTH the message and that the refusal comes from the ADAPTER, not
from pandas — the second half is the actual contract, and asserting the message
alone would pass just as happily if a future change made pandas raise it instead.
"""

from __future__ import annotations

import pytest


def _ask(oracle, payload: dict):
    """Run one operation through the dispatcher and return the raised OracleError."""
    with pytest.raises(oracle.OracleError) as excinfo:
        oracle.dispatch(__import__("pandas"), payload)
    return excinfo.value


@pytest.mark.parametrize(
    ("operation", "payload", "fragment"),
    [
        (
            "dataframe_from_series",
            {},
            "requires at least one series payload",
        ),
        (
            "dataframe_constructor_dict_of_series",
            {},
            "requires at least one series payload",
        ),
        (
            "dataframe_constructor_kwargs",
            {},
            "requires frame payload",
        ),
        (
            "dataframe_constructor_scalar",
            {"index": [{"kind": "int64", "value": 0}]},
            "requires fill_value payload",
        ),
        (
            "dataframe_constructor_list_like",
            {},
            "requires matrix_rows list payload",
        ),
    ],
)
def test_adapter_refuses_a_missing_payload_with_its_own_message(
    oracle, operation, payload, fragment
):
    error = _ask(oracle, dict(payload, operation=operation))
    assert fragment in str(error), (
        f"{operation} should refuse a missing payload with {fragment!r}; got {error!r}"
    )
    # THE SECOND HALF IS THE CONTRACT. A message assertion alone would pass just as
    # happily if pandas started raising it — and an adapter refusal credited to
    # pandas is precisely what error_origin exists to prevent
    # (br-frankenpandas-fixture-corpus-stale-vs-oracle-p6srr).
    assert oracle.oracle_error_origin(error) == oracle.ERROR_ORIGIN_ADAPTER, (
        f"{operation}'s missing-payload refusal must be attributed to the ADAPTER; "
        f"pandas was never invoked because there is nothing to pass it"
    )


def test_the_adapter_origin_constant_is_not_the_pandas_one() -> None:
    """Non-vacuity for the origin half of every assertion above.

    If those two constants were ever equal, the `classify_error_origin` check would
    hold for an adapter refusal AND for a genuine pandas raise, and the tests would
    be asserting nothing about attribution.
    """
    import pandas  # noqa: F401  (import proves the suite's pandas is importable)

    from pandas_oracle import ERROR_ORIGIN_ADAPTER, ERROR_ORIGIN_PANDAS

    assert ERROR_ORIGIN_ADAPTER != ERROR_ORIGIN_PANDAS
