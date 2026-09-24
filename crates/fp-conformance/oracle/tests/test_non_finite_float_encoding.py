"""±inf is encoded as the strings "inf"/"-inf", the spelling fp-types reads.

History (br-frankenpandas-oracle-float-label-asymmetry-ab1gd): `json.dumps` writes
a bare `Infinity` token, which is not JSON; serde_json rejects it and the whole
packet aborts, and `series_div` by zero reaches it through the ordinary
dispatcher. Until both sides agreed on a spelling, the oracle refused. The spelling
now exists on both sides (br-frankenpandas-rc0923-epic-rust-parity-bugs-4qg5w.3):

    Scalar::Float64(f64::INFINITY)  <->  {"kind": "float64", "value": "inf"}

(pinned on the Rust side by fp-types `float64_json_spells_infinity_and_round_trips`,
which parses these exact bytes). An infinity stays a float64 VALUE: routing it to a
null kind would claim an infinite result is missing.
"""

from __future__ import annotations

import json
import math

import pytest


def _strict_loads(raw: str):
    """`json.loads` that REJECTS the bare NaN/Infinity tokens Python accepts.

    Plain `json.loads` is lenient and parses them happily, which is exactly why
    the original hole survived: a Python-only round trip passed while serde_json
    refused the same bytes.
    """

    def reject(token):
        raise ValueError(f"bare non-JSON constant {token!r}")

    return json.loads(raw, parse_constant=reject)


@pytest.mark.parametrize("value, spelling", [(math.inf, "inf"), (-math.inf, "-inf")])
def test_scalar_to_json_spells_non_finite_floats(oracle, value, spelling):
    assert oracle.scalar_to_json(value) == {"kind": "float64", "value": spelling}


def test_finite_and_nan_floats_keep_their_encodings(oracle):
    """NaN already had a working encoding (a typed null) that must not move."""
    assert oracle.scalar_to_json(1.5) == {"kind": "float64", "value": 1.5}
    assert oracle.scalar_to_json(math.nan) == {"kind": "null", "value": "na_n"}


def test_every_float_encoding_is_strict_json_and_reads_back(oracle):
    """The property at stake is 'parses where it will be parsed' (serde_json).
    The old bare-token form stays as the counter-example."""
    for value in (1.5, -0.25, math.nan, math.inf, -math.inf):
        encoded = oracle.scalar_to_json(value)
        decoded = _strict_loads(json.dumps(encoded))
        back = oracle.scalar_from_json(decoded)
        if math.isnan(value):
            assert math.isnan(back)
        else:
            assert back == value

    with pytest.raises(ValueError, match="bare non-JSON constant"):
        _strict_loads(json.dumps({"kind": "float64", "value": math.inf}))


def test_series_div_by_zero_answers_with_infinity(oracle):
    """The reachable path that used to abort: pandas' 1.0 / 0.0 == inf and
    -1.0 / 0.0 == -inf now come back as encodable values."""
    pd = pytest.importorskip("pandas")
    payload = {
        "name": "a",
        "index": [{"kind": "int64", "value": 0}, {"kind": "int64", "value": 1}],
        "values": [{"kind": "float64", "value": 1.0}, {"kind": "float64", "value": -1.0}],
    }
    zero = dict(
        payload,
        values=[{"kind": "float64", "value": 0.0}, {"kind": "float64", "value": 0.0}],
    )
    result = oracle.dispatch(pd, {"operation": "series_div", "left": payload, "right": zero})
    values = _strict_loads(json.dumps(result))["expected_series"]["values"]
    assert values == [
        {"kind": "float64", "value": "inf"},
        {"kind": "float64", "value": "-inf"},
    ]
