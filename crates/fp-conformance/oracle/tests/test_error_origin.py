"""Where an oracle refusal came from — pandas, or this adapter first.

br-frankenpandas-fixture-corpus-stale-vs-oracle-p6srr: 85 fixtures in the corpus
assert an operation fails, and the current oracle also fails on them, so they
look like one homogeneous "both sides agree" tranche that could be restamped in
bulk. It is not homogeneous. Roughly a third of those refusals never reached
pandas at all — the adapter rejected the arguments itself — and for those,
"this oracle also failed here" is true but says nothing whatsoever about pandas.

`oracle_error_origin` is the discriminator. It must be structural, not a
substring match on the message text, because the message is exactly the thing
that drifts.
"""
from __future__ import annotations

import json

import pytest


def test_a_bare_oracle_error_is_the_adapter_refusing(oracle):
    """`raise OracleError("... requires ... payload")` — pandas never invoked."""
    exc = oracle.OracleError("series_join requires join_type=inner|left|right|outer")
    assert oracle.oracle_error_origin(exc) == oracle.ERROR_ORIGIN_ADAPTER


def test_a_wrapped_engine_exception_is_pandas(oracle):
    """`raise OracleError(...) from exc` around a pandas call — pandas refused."""
    try:
        try:
            raise IndexError("positional indexers are out-of-bounds")
        except Exception as inner:  # noqa: BLE001
            raise oracle.OracleError(f"dataframe_iloc failed: {inner}") from inner
    except oracle.OracleError as exc:
        assert oracle.oracle_error_origin(exc) == oracle.ERROR_ORIGIN_PANDAS
    else:  # pragma: no cover
        pytest.fail("expected OracleError")


def test_a_malformed_request_is_neither(oracle):
    """The stdin decode wrapper has a __cause__ but pandas is not implicated."""
    try:
        try:
            json.loads("{not json")
        except json.JSONDecodeError as inner:
            raise oracle.OracleError(f"invalid oracle request JSON: {inner}") from inner
    except oracle.OracleError as exc:
        assert oracle.oracle_error_origin(exc) == oracle.ERROR_ORIGIN_REQUEST
    else:  # pragma: no cover
        pytest.fail("expected OracleError")


def test_the_origins_are_distinct_labels(oracle):
    """A collision would silently merge attestable and non-attestable rows."""
    origins = {
        oracle.ERROR_ORIGIN_PANDAS,
        oracle.ERROR_ORIGIN_ADAPTER,
        oracle.ERROR_ORIGIN_REQUEST,
        oracle.ERROR_ORIGIN_UNEXPECTED,
    }
    assert len(origins) == 4


def test_error_response_carries_the_origin_and_still_stamps_provenance(oracle, pd):
    """Both halves of what made these fixtures unstampable.

    The provenance is what a restamp needs; the origin is what decides whether
    restamping is honest. An error response must carry both.
    """
    response = oracle.error_response(
        "dataframe_iloc failed: positional indexers are out-of-bounds",
        pd,
        oracle.ERROR_ORIGIN_PANDAS,
    )
    assert response["error_origin"] == oracle.ERROR_ORIGIN_PANDAS
    provenance = response["fixture_provenance"]
    assert provenance["oracle_script_sha256"] == oracle.oracle_script_sha256()
    assert provenance["pandas_version"] == str(pd.__version__)


def test_origin_defaults_to_unexpected_rather_than_pandas(oracle, pd):
    """Fail CLOSED: an unclassified refusal must not borrow pandas' authority.

    ERROR_ORIGIN_PANDAS is the only value that unlocks an attestation, so the
    default has to be anything else.
    """
    response = oracle.error_response("something went wrong", pd)
    assert response["error_origin"] == oracle.ERROR_ORIGIN_UNEXPECTED
    assert response["error_origin"] != oracle.ERROR_ORIGIN_PANDAS


def test_a_success_response_has_no_origin(oracle):
    """The key exists on every response so consumers need no special case."""
    assert oracle.base_oracle_response()["error_origin"] is None


def test_an_oracle_error_wrapping_an_oracle_error_is_still_the_adapter(oracle):
    """The trap: a helper's OWN refusal, re-wrapped by an op handler.

    `pandas_dtype_from_constructor_spec` raises a bare OracleError inside
    op_dataframe_constructor_list_like's try-block, which re-raises it with
    `from exc`. Judging on __cause__ alone credits PANDAS for a rejection pandas
    never saw — observed live on
    fp_p2d_024_..._dtype_boolean_pyarrow_unsupported_error_hardened, which
    classified as `pandas` before this was fixed.
    """
    try:
        try:
            raise oracle.OracleError("unsupported constructor dtype 'boolean[pyarrow]'")
        except oracle.OracleError as inner:
            raise oracle.OracleError(f"dataframe_constructor_list_like failed: {inner}") from inner
    except oracle.OracleError as exc:
        assert oracle.oracle_error_origin(exc) == oracle.ERROR_ORIGIN_ADAPTER
    else:  # pragma: no cover
        pytest.fail("expected OracleError")


def test_an_oracle_error_wrapping_a_wrapped_engine_error_is_still_pandas(oracle):
    """The unwrap must stop at the ROOT, not at the first OracleError.

    A genuine pandas exception nested under two adapter layers is still pandas;
    over-correcting would strip attestation from the 49 rows that earned it.
    """
    try:
        try:
            try:
                raise ValueError("Need to pass bool-like values")
            except ValueError as engine:
                raise oracle.OracleError(f"cast failed: {engine}") from engine
        except oracle.OracleError as inner:
            raise oracle.OracleError(f"constructor failed: {inner}") from inner
    except oracle.OracleError as exc:
        assert oracle.oracle_error_origin(exc) == oracle.ERROR_ORIGIN_PANDAS
    else:  # pragma: no cover
        pytest.fail("expected OracleError")
