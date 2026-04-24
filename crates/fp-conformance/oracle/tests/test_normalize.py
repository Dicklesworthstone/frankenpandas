"""Unit tests for pandas_oracle.py scalar / label normalization helpers.

Per br-frankenpandas-urhy: the normalize_* + scalar/label helpers are
the first layer where fixture JSON meets pandas values. A bug here
produces silently-wrong fixtures across every op handler.
"""
from __future__ import annotations

import math


def test_scalar_to_json_handles_none(oracle):
    assert oracle.scalar_to_json(None) == {"kind": "null", "value": "null"}


def test_scalar_to_json_handles_nan(oracle):
    result = oracle.scalar_to_json(float("nan"))
    assert result == {"kind": "null", "value": "na_n"}


def test_scalar_to_json_bool_not_collapsed_to_int(oracle):
    # isinstance(True, int) is True in Python; oracle must check bool first.
    assert oracle.scalar_to_json(True) == {"kind": "bool", "value": True}
    assert oracle.scalar_to_json(False) == {"kind": "bool", "value": False}


def test_scalar_to_json_int(oracle):
    assert oracle.scalar_to_json(42) == {"kind": "int64", "value": 42}
    assert oracle.scalar_to_json(-1) == {"kind": "int64", "value": -1}


def test_scalar_to_json_float(oracle):
    result = oracle.scalar_to_json(3.14)
    assert result == {"kind": "float64", "value": 3.14}


def test_scalar_to_json_string(oracle):
    assert oracle.scalar_to_json("hello") == {"kind": "utf8", "value": "hello"}


def test_scalar_from_json_null_variants(oracle):
    assert oracle.scalar_from_json({"kind": "null", "value": "null"}) is None
    nan_result = oracle.scalar_from_json({"kind": "null", "value": "nan"})
    assert isinstance(nan_result, float) and math.isnan(nan_result)
    nan_result2 = oracle.scalar_from_json({"kind": "null", "value": "na_n"})
    assert isinstance(nan_result2, float) and math.isnan(nan_result2)


def test_scalar_from_json_primitive_kinds(oracle):
    assert oracle.scalar_from_json({"kind": "bool", "value": True}) is True
    assert oracle.scalar_from_json({"kind": "int64", "value": 7}) == 7
    assert oracle.scalar_from_json({"kind": "float64", "value": 1.5}) == 1.5
    assert oracle.scalar_from_json({"kind": "utf8", "value": "x"}) == "x"


def test_round_trip_scalar_preserves_value(oracle):
    for value in [0, 1, -5, 3.14, True, False, "abc", None]:
        json_form = oracle.scalar_to_json(value)
        back = oracle.scalar_from_json(json_form)
        if value is None:
            assert back is None
        elif isinstance(value, float) and math.isnan(value):
            assert isinstance(back, float) and math.isnan(back)
        else:
            assert back == value


def test_label_to_json_int_string_bool(oracle):
    # bool is tested before int (isinstance(True, int) is True)
    assert oracle.label_to_json(True) == {"kind": "bool", "value": True}
    assert oracle.label_to_json(42) == {"kind": "int64", "value": 42}
    assert oracle.label_to_json("idx") == {"kind": "utf8", "value": "idx"}


def test_label_from_json_parses_all_declared_kinds(oracle):
    assert oracle.label_from_json({"kind": "bool", "value": True}) is True
    assert oracle.label_from_json({"kind": "int64", "value": 3}) == 3
    assert oracle.label_from_json({"kind": "float64", "value": 1.0}) == 1.0
    assert oracle.label_from_json({"kind": "utf8", "value": "k"}) == "k"


def test_label_from_json_rejects_unknown_kind(oracle):
    import pytest

    with pytest.raises(oracle.OracleError):
        oracle.label_from_json({"kind": "rocket", "value": "🚀"})


def test_fixture_provenance_records_pandas_version_and_oracle_hash(oracle, pd):
    provenance = oracle.build_fixture_provenance(pd)

    assert provenance["pandas_version"] == pd.__version__
    assert len(provenance["oracle_script_sha256"]) == 64
    assert provenance["generated_at"].endswith("Z")


def test_error_response_keeps_fixture_provenance_when_pandas_loaded(oracle, pd):
    response = oracle.error_response("bad request", pd)

    assert response["error"] == "bad request"
    assert response["fixture_provenance"]["pandas_version"] == pd.__version__
    assert response["expected_series"] is None
