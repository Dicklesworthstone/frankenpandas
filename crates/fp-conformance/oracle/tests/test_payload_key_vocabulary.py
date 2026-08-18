"""The two arms must be asked the SAME question.

br-frankenpandas: the oracle reads payload keys the Rust `PacketFixture` cannot
deserialise. `dt_strftime_format` is the worked example — the oracle also honours
`dt_format` as a fallback alias, and the Rust struct has no such alias, so a
fixture spelling it the oracle's way hands pandas the requested format and
FrankenPandas the DEFAULT. The arms then compute different things and the
mismatch is reported as a parity failure, which is the most misleading possible
symptom: it looks like the engine diverging when it is the QUESTION diverging.

MEASURED when this test was written: 65 such keys exist, and ZERO fixtures use any
of them. The hazard is latent, not live. This test exists to keep it that way —
the cost of the mismatch is paid by whoever writes the next fixture, and they will
read it as a real divergence exactly as I did.

Deliberately checks the FIXTURES rather than diffing the two vocabularies. An
oracle-only key is not by itself a bug (dead paths and nested payloads both
produce them); a fixture USING one is. This asserts the thing that matters and
does not fail on the thing that does not.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
ORACLE = REPO / "crates/fp-conformance/oracle/pandas_oracle.py"
RUST = REPO / "crates/fp-conformance/src/lib.rs"
PACKETS = REPO / "crates/fp-conformance/fixtures/packets"

# Keys that live in the request ENVELOPE rather than in a fixture's knob set.
ENVELOPE = {
    "left", "right", "frame", "index", "values", "columns", "operation",
    "packet_id", "case_id", "mode", "fixture_provenance", "name", "kind",
    "value", "column_order", "groupby_keys", "expected_series", "expected_frame",
}


def _oracle_payload_keys() -> set[str]:
    src = ORACLE.read_text()
    keys: set[str] = set()
    keys |= set(re.findall(r'payload\.get\(\s*"([a-z0-9_]+)"', src))
    keys |= set(re.findall(r'required_string_payload\(payload,\s*"([a-z0-9_]+)"', src))
    keys |= set(re.findall(r'parse_optional_string_list\(payload,\s*"([a-z0-9_]+)"', src))
    keys |= set(re.findall(r'optional_float_payload\(payload,\s*"([a-z0-9_]+)"', src))
    return keys - ENVELOPE


def _rust_fixture_fields() -> set[str]:
    src = RUST.read_text()
    start = src.index("pub struct PacketFixture")
    body = src[start : src.index("\n}", start)]
    fields = set(re.findall(r"\n    pub ([a-z0-9_]+):", body))
    fields |= set(re.findall(r'rename\s*=\s*"([a-z0-9_]+)"', body))
    fields |= set(re.findall(r'alias\s*=\s*"([a-z0-9_]+)"', body))
    return fields


def test_no_fixture_uses_a_key_only_one_arm_understands() -> None:
    oracle_only = _oracle_payload_keys() - _rust_fixture_fields()
    # Non-vacuity: if this set is empty the assertion below cannot fail, and the
    # test would pass while checking nothing. `dt_format` is the known member.
    assert oracle_only, (
        "expected at least one oracle-only payload key; if the two vocabularies "
        "have genuinely converged, delete this test rather than letting it pass "
        "vacuously"
    )

    offenders: dict[str, list[str]] = {}
    for path in sorted(PACKETS.glob("*.json")):
        try:
            fixture = json.loads(path.read_text())
        except Exception:  # a malformed fixture is another test's problem
            continue
        for key in fixture:
            if key in oracle_only:
                offenders.setdefault(key, []).append(path.name)

    assert not offenders, (
        "these fixtures spell a payload key the ORACLE reads and the RUST harness "
        "cannot deserialise, so the two arms are being asked DIFFERENT QUESTIONS "
        "and any mismatch will look like a parity divergence:\n"
        + "\n".join(f"  {key}: {files}" for key, files in sorted(offenders.items()))
        + "\nEither rename the key to the Rust spelling or add the alias to "
        "PacketFixture — do not 'fix' the resulting expectation."
    )


def test_the_known_alias_hazard_is_still_present_and_named() -> None:
    """`dt_format` is the worked example; keep it findable.

    If this fails because the Rust side gained the alias, that is a FIX — delete
    this test and note it. If it fails because the oracle dropped the fallback,
    that is also a fix. Either way the failure should be read before being made to
    pass.
    """
    src = ORACLE.read_text()
    assert 'payload.get("dt_format"' in src, (
        "the oracle no longer honours the `dt_format` fallback. If that was "
        "deliberate, this test has done its job and can go."
    )
    assert "dt_format" not in _rust_fixture_fields(), (
        "PacketFixture now accepts `dt_format`, so the two arms agree on it and "
        "this specific hazard is closed"
    )
