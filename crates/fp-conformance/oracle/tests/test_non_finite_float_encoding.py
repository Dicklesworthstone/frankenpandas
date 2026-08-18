"""±inf has no JSON encoding, so the oracle must REFUSE it rather than emit it.

br-frankenpandas-oracle-float-label-asymmetry-ab1gd flagged this as a latent hole
in `scalar_to_json`. Measuring it upgraded it twice over:

REACHABLE, not hypothetical. `series_div` with a zero denominator produces an
infinite value through the ordinary dispatcher today — no exotic input required.

PACKET-WIDE, not fixture-local. `json.dumps` writes the bare token `Infinity`,
which is not JSON. Feeding fp-conformance-cli a fixture carrying it gives

    Error: Json(Error("expected value", line: 8, column: 112))

and the run produces NO per-fixture results at all — one unparseable fixture takes
down every sibling in its packet, so the blast radius of emitting it is far larger
than the fixture that contains it.

⚠️ THE FIX REFUSES; IT DOES NOT INVENT A SPELLING. There is no encoding for ±inf
that both sides accept. The Rust `NullKind` is Null/NaN/NaT with no Inf, so routing
an infinity to a null kind would assert that an infinite value is MISSING — false,
and precisely the quiet wrong answer this repo keeps choosing loudness over.
Picking a real spelling requires a matching Rust-side change and belongs with
ab1gd's batched emitter work, which is blocked on p6srr. Refusing is what can be
done honestly today.

MEASURED RADIUS: dispatching all 1277 corpus fixtures through the oracle in-process,
ZERO hit the new refusal. (The packet run does not exercise this at all — it
compares FrankenPandas against STORED expectations and never calls the emitter — so
the dispatch sweep is the measurement that actually applies.)
"""

from __future__ import annotations

import json
import math

import pytest


def _strict_loads(raw: str):
    """`json.loads` that REJECTS the bare NaN/Infinity tokens Python accepts.

    Plain `json.loads` is lenient and parses them happily, which is exactly why
    this hole survived: a round-trip test written in Python alone would pass while
    serde_json on the Rust side refused the same bytes.
    """

    def reject(token):
        raise ValueError(f"bare non-JSON constant {token!r}")

    return json.loads(raw, parse_constant=reject)


@pytest.mark.parametrize("value", [math.inf, -math.inf])
def test_scalar_to_json_refuses_non_finite_floats_ab1gd(oracle, value):
    with pytest.raises(oracle.OracleError) as excinfo:
        oracle.scalar_to_json(value)
    message = str(excinfo.value)
    assert "non-finite float value" in message, (
        "the refusal must name what it refused; a generic error here sends readers "
        "looking for a broken operation instead of an unencodable value"
    )
    assert "ab1gd" in message, (
        "the message must point at the bead that owns the spelling decision, so "
        "whoever hits it knows this is undecided rather than unimplementable"
    )


def test_finite_and_nan_floats_still_encode_ab1gd(oracle):
    """Non-vacuity. Without this the refusal could be 'achieved' by rejecting every
    float, and NaN in particular already HAS a working encoding that must not move.
    """
    assert oracle.scalar_to_json(1.5) == {"kind": "float64", "value": 1.5}
    assert oracle.scalar_to_json(math.nan) == {"kind": "null", "value": "na_n"}


def test_the_encodings_that_survive_are_strict_json_ab1gd(oracle):
    """Pins the actual property at stake, which is not 'no exception' but 'parses
    where it will be parsed'. The old inf form is included as the counter-example
    so the test states what it is protecting against, not just what it allows.
    """
    for value in (1.5, -0.25, math.nan):
        raw = json.dumps(oracle.scalar_to_json(value))
        assert _strict_loads(raw) is not None

    # What the emitter used to produce for an infinity, verified to be the thing
    # serde_json refuses. This is a fixed literal rather than a call, because the
    # emitter can no longer be made to produce it.
    with pytest.raises(ValueError, match="bare non-JSON constant"):
        _strict_loads(json.dumps({"kind": "float64", "value": math.inf}))


def test_series_div_by_zero_is_the_reachable_path_ab1gd(oracle):
    """The reachability claim, asserted rather than described.

    If a future change gives ±inf a real encoding, this test fails and points at
    the one place that has to agree with it — which is the right outcome, not a
    nuisance: the refusal and the reachable operation must be decided together.
    """
    payload = {
        "name": "a",
        "index": [{"kind": "int64", "value": 0}],
        "values": [{"kind": "float64", "value": 1.0}],
    }
    zero = dict(payload, values=[{"kind": "float64", "value": 0.0}])
    with pytest.raises(oracle.OracleError, match="non-finite float value"):
        oracle.dispatch(
            __import__("pandas"),
            {"operation": "series_div", "left": payload, "right": zero},
        )
