"""The corpus-wide `--apply` ban, and why `--restamp-agreeing` is exempt from it.

br-frankenpandas-fixture-corpus-stale-vs-oracle-p6srr.

`regenerate_fixtures.py` refuses a corpus-wide `--apply`, because a bulk sweep
that pulls expected values from the oracle is how a divergence gets laundered
into the corpus as pinned truth. That ban must stay.

`--restamp-agreeing` is a different operation wearing the same flag. It reads no
expected value at all — `restamp` copies `fixture_provenance` and nothing else —
and it only fires where `restampable` proved the current oracle reproduced
everything the fixture pins. p6srr is 1144 such fixtures, so the mode is
worthless if it cannot run corpus-wide, and `restamp`'s own docstring calls
itself "the honest remedy for the bulk of p6srr".

These tests pin BOTH halves, because the exemption is only safe while the second
half holds:

  1. the ban still fires for value regeneration, and
  2. restamping provably cannot change an expected value.

The obvious way to "fix" the blocked restamp was to batch it behind `--limit
400` three times. That satisfies the guard's letter while doing exactly what it
forbids, so the guard was narrowed in the open and pinned here instead.
"""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts" / "regenerate_fixtures.py"


def _load():
    spec = importlib.util.spec_from_file_location("regenerate_fixtures", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    sys.modules["regenerate_fixtures"] = module
    spec.loader.exec_module(module)
    return module


def _run(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        capture_output=True,
        text=True,
        cwd=REPO,
        timeout=120,
    )


def test_corpus_wide_value_regeneration_is_still_refused():
    """The ban this exemption must not weaken."""
    result = _run("--apply", "--attributions", "/nonexistent-allowlist.json")
    assert result.returncode == 2, result.stdout + result.stderr
    assert "corpus-wide regeneration is forbidden" in result.stderr


def test_restamp_agreeing_is_exempt_from_the_corpus_wide_ban():
    """...and the exemption is reachable, or the restamp mode is dead code.

    Asserted against the rule itself rather than by invoking the CLI: the only
    argument shape that exercises this is corpus-wide by definition, so a
    subprocess test would have to sweep all 1259 fixtures to check one boolean.
    """
    module = _load()
    refused = module.corpus_wide_apply_refused

    # Value regeneration, corpus-wide: REFUSED.
    assert refused(apply=True, restamp_agreeing=False, glob="*.json", limit=0)
    # The same shape, restamping only: ALLOWED.
    assert not refused(apply=True, restamp_agreeing=True, glob="*.json", limit=0)
    # Scoping by glob or limit still lets regeneration through, as before.
    assert not refused(apply=True, restamp_agreeing=False, glob="fp_p2d_017_*.json", limit=0)
    assert not refused(apply=True, restamp_agreeing=False, glob="*.json", limit=25)
    # A dry run is never refused.
    assert not refused(apply=False, restamp_agreeing=False, glob="*.json", limit=0)


def test_restamp_copies_provenance_and_nothing_else():
    """The property the exemption rests on: expected values cannot move."""
    module = _load()
    fixture = {
        "packet_id": "FP-TEST-001",
        "case_id": "restamp_probe",
        "operation": "series_head",
        "fixture_provenance": {
            "pandas_version": "2.2.3",
            "oracle_script_sha256": "0" * 64,
            "generated_at": "2026-04-22T21:02:48Z",
        },
        "left": {"values": [{"kind": "int64", "value": 1}]},
        "expected_series": {
            "index": [{"kind": "int64", "value": 0}],
            "values": [{"kind": "int64", "value": 1}],
        },
    }
    # A response that would REWRITE the expected values if restamp read them.
    response = {
        "fixture_provenance": {
            "pandas_version": "2.2.3",
            "oracle_script_sha256": "f" * 64,
            "generated_at": "2026-08-16T00:00:00Z",
        },
        "expected_series": {
            "index": [{"kind": "int64", "value": 99}],
            "values": [{"kind": "int64", "value": 99}],
        },
    }
    out = module.restamp(fixture, response)

    assert out["fixture_provenance"]["oracle_script_sha256"] == "f" * 64
    assert out["expected_series"] == fixture["expected_series"], (
        "restamp took an expected value from the oracle response — that is "
        "regeneration, and the corpus-wide exemption is unsafe if it can happen"
    )
    for key in fixture:
        if key != "fixture_provenance":
            assert out[key] == fixture[key], f"restamp altered {key!r}"


def test_restampable_refuses_a_fixture_nothing_verified():
    """A green-looking fixture whose keys were never compared is NOT restampable.

    Stamping it would assert "this oracle produced these values" over a claim
    nothing checked — the silent-non-comparison-as-success bug, committed into
    the corpus as provenance.
    """
    module = _load()
    assert module.restampable({"moved": [], "uncompared": [], "how": {"expected_series": "semantic"}})
    assert not module.restampable({"moved": [], "uncompared": [], "how": {}}), (
        "zero compared keys must not be restampable"
    )
    assert not module.restampable({"moved": [], "uncompared": ["expected_series"], "how": {"x": "y"}})
    assert not module.restampable({"moved": ["expected_series"], "uncompared": [], "how": {"x": "y"}})


def test_restamp_text_leaves_the_rest_of_the_file_byte_identical():
    """Restamping ~1000 files must stay reviewable, so it is a textual splice.

    A `json.dumps(indent=2)` round-trip would reformat every fixture it touched
    and bury a 3-line change under ~60 lines of noise per file.
    """
    module = _load()
    raw = json.dumps(
        {
            "case_id": "x",
            "fixture_provenance": {
                "pandas_version": "2.2.3",
                "oracle_script_sha256": "0" * 64,
                "generated_at": "2026-04-22T21:02:48Z",
            },
            "expected_series": {"values": [{"kind": "int64", "value": 1}]},
        },
        indent=2,
    )
    new = module.restamp_text(
        raw,
        {"oracle_script_sha256": "0" * 64, "generated_at": "2026-04-22T21:02:48Z"},
        {"oracle_script_sha256": "a" * 64, "generated_at": "2026-08-16T00:00:00Z"},
    )
    before, after = json.loads(raw), json.loads(new)
    assert after["fixture_provenance"]["oracle_script_sha256"] == "a" * 64
    assert after["expected_series"] == before["expected_series"]
    # The old hash must be gone, not merely accompanied by the new one.
    assert "0" * 64 not in new


def test_appending_an_attestation_keeps_the_closing_brace_indented():
    """A new provenance key must not deform the block it is appended to.

    `restamp_text` splices rather than re-dumping precisely so the diff stays
    reviewable. The first version dropped the closing brace's indentation when it
    appended `oracle_attestation`, which is still valid JSON — so nothing failed
    and the exit code was 0 — while silently reformatting every fixture it
    touched. It reached 71 files before a human read the diff.
    """
    module = _load()
    raw = (
        '{\n'
        '  "case_id": "x",\n'
        '  "fixture_provenance": {\n'
        '    "pandas_version": "2.2.3",\n'
        '    "oracle_script_sha256": "' + "0" * 64 + '",\n'
        '    "generated_at": "2026-04-22T21:02:48Z"\n'
        '  },\n'
        '  "expected_error_contains": "boom"\n'
        '}\n'
    )
    new = module.restamp_text(
        raw,
        {"oracle_script_sha256": "0" * 64, "generated_at": "2026-04-22T21:02:48Z"},
        {
            "oracle_script_sha256": "a" * 64,
            "generated_at": "2026-08-16T00:00:00Z",
            "oracle_attestation": "error_agreement",
        },
    )
    assert '\n  },\n' in new, (
        "the provenance block's closing brace lost its indentation:\n" + new
    )
    assert "\n},\n" not in new
    parsed = json.loads(new)
    assert parsed["fixture_provenance"]["oracle_attestation"] == "error_agreement"
    assert parsed["expected_error_contains"] == "boom"


# ---------------------------------------------------------------------------
# br-frankenpandas-1dbxe: a refused restamp must say WHICH guard refused it.
#
# The bead is a P1 alleging that `--jobs 8 --restamp-agreeing --apply` silently
# re-banked an expected value ("Sparse" -> "Sparse[int64, 0]") in a fixture the
# same run reported as MOVED. Reading the writer settles the mechanism: it never
# serializes the parsed fixture at all. It reads the file's RAW TEXT, splices
# only inside the brace-matched `fixture_provenance` block, and writes that text
# back — so there is no code path that can author a different `expected_dtype`.
# A peer's uncommitted working-tree edit explains the observed diff, and the
# shared checkout makes "the tool wrote this" and "a peer wrote this"
# indistinguishable from the diff alone.
#
# What WAS a real defect: every refusal printed one hardcoded explanation
# ("richer provenance than the oracle emits") no matter which of four guards
# fired. So the one output that could have separated those two stories said the
# wrong thing. These tests pin the reasons.
# ---------------------------------------------------------------------------


def _probe_fixture() -> dict:
    return {
        "packet_id": "FP-TEST-002",
        "case_id": "refusal_reason_probe",
        "operation": "series_head",
        "fixture_provenance": {
            "pandas_version": "2.2.3",
            "oracle_script_sha256": "0" * 64,
            "generated_at": "2026-04-22T21:02:48Z",
        },
        "expected_dtype": "Sparse",
    }


def _fresh_response() -> dict:
    return {
        "fixture_provenance": {
            "pandas_version": "2.2.3",
            "oracle_script_sha256": "a" * 64,
            "generated_at": "2026-08-16T00:00:00Z",
        }
    }


def test_write_restamp_reports_success_as_a_pair_and_refreshes_only_provenance(tmp_path):
    """The happy path, and the shape every caller branches on."""
    module = _load()
    path = tmp_path / "probe.json"
    fixture = _probe_fixture()
    path.write_text(json.dumps(fixture, indent=2) + "\n", encoding="utf-8")

    written, why = module.write_restamp(path, fixture, _fresh_response())

    assert written is True
    assert why is None
    landed = json.loads(path.read_text(encoding="utf-8"))
    assert landed["fixture_provenance"]["oracle_script_sha256"] == "a" * 64
    assert module.non_provenance(landed) == module.non_provenance(fixture)


def test_a_file_edited_after_it_was_examined_is_refused_as_a_stale_snapshot(tmp_path):
    """THE 1dbxe SCENARIO, reproduced without the oracle or a corpus run.

    `fixture` is the snapshot `examine()` parsed — in a worker thread, possibly
    minutes earlier under `--jobs N`. Between then and the write, something else
    changes the file: a peer editing the shared checkout, which is exactly what
    the bead's alternative explanation describes.

    The old code refused this too, via the non-provenance comparison, so the
    corpus was never at risk. But it reported the refusal as a provenance
    problem, which sent an adjudicator hunting a concurrency bug in the writer.
    The refusal must now NAME the skew.
    """
    module = _load()
    path = tmp_path / "probe.json"
    examined = _probe_fixture()
    path.write_text(json.dumps(examined, indent=2) + "\n", encoding="utf-8")

    # Somebody else moves the expected value while the run is in flight.
    on_disk = _probe_fixture()
    on_disk["expected_dtype"] = "Sparse[int64, 0]"
    path.write_text(json.dumps(on_disk, indent=2) + "\n", encoding="utf-8")

    written, why = module.write_restamp(path, examined, _fresh_response())

    assert written is False
    assert why == module.REFUSED_STALE_SNAPSHOT, why
    assert "stale-snapshot" in why
    # And the peer's edit survives untouched: refusing means refusing to write,
    # not reverting somebody else's file.
    assert json.loads(path.read_text(encoding="utf-8"))["expected_dtype"] == "Sparse[int64, 0]"


def test_a_stale_snapshot_is_not_reported_as_a_provenance_richness_problem(tmp_path):
    """NEGATIVE CASE: the two refusals must not collapse into one message.

    This is the assertion the old code fails. It returned a bare `False` and the
    summary printed "richer provenance than the oracle emits" for every refusal,
    so a concurrent edit and a genuinely rich stamp were the same output. A test
    that only checked `written is False` passes against that code and proves
    nothing; the reasons have to be DISTINCT.
    """
    module = _load()

    stale_path = tmp_path / "stale.json"
    examined = _probe_fixture()
    stale_path.write_text(json.dumps(examined, indent=2) + "\n", encoding="utf-8")
    moved = _probe_fixture()
    moved["expected_dtype"] = "Sparse[int64, 0]"
    stale_path.write_text(json.dumps(moved, indent=2) + "\n", encoding="utf-8")
    _, stale_why = module.write_restamp(stale_path, examined, _fresh_response())

    # A fixture carrying provenance the oracle's three-key stamp cannot express.
    rich_path = tmp_path / "rich.json"
    rich = _probe_fixture()
    rich["fixture_provenance"]["intentional_divergence_notes"] = "DISC-026"
    rich_path.write_text(json.dumps(rich, indent=2) + "\n", encoding="utf-8")
    # A response whose provenance DROPS the extra key rather than preserving it.
    dropping = {"fixture_provenance": {"pandas_version": "2.2.3"}}
    rich_written, rich_why = module.write_restamp(rich_path, rich, dropping)

    assert stale_why == module.REFUSED_STALE_SNAPSHOT
    assert stale_why != rich_why, (
        "a concurrent edit and a rich-provenance refusal produced the SAME "
        "message — that conflation is what made br-frankenpandas-1dbxe read as "
        "silent laundering in the tool"
    )
    if not rich_written:
        assert rich_why is not None and "stale-snapshot" not in rich_why


def test_the_writer_cannot_author_an_expected_value_it_was_not_given(tmp_path):
    """The structural fact underneath 1dbxe, pinned so it stays true.

    `write_restamp` splices text inside the `fixture_provenance` block only. Hand
    it a response carrying a DIFFERENT expected value and the written file must
    keep the fixture's own — not because a guard caught it, but because no code
    path reads an expected value from the response at all.
    """
    module = _load()
    path = tmp_path / "probe.json"
    fixture = _probe_fixture()
    path.write_text(json.dumps(fixture, indent=2) + "\n", encoding="utf-8")

    response = _fresh_response()
    response["expected_dtype"] = "Sparse[int64, 0]"

    written, why = module.write_restamp(path, fixture, response)

    assert written is True, why
    landed = json.loads(path.read_text(encoding="utf-8"))
    assert landed["expected_dtype"] == "Sparse", (
        "the writer took an expected value from the oracle response — that is "
        "regeneration, and it is the exact harm br-frankenpandas-1dbxe alleges"
    )
