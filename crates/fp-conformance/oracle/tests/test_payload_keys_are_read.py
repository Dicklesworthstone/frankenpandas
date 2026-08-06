"""Every option a fixture carries must be one the oracle actually reads.

Per br-frankenpandas-fixture-divergence-triage-9s0c4. The oracle takes its
arguments out of a JSON payload by string key. When a fixture spells an option
one way and the oracle reads another, there is no error and no warning — the
oracle silently falls back to a default and evaluates a DIFFERENT operation than
the fixture asked for. The fixture then either diverges for a reason nobody can
attribute, or, worse, agrees by accident because the default happens to match.

Both failure modes were live in this repo:

  dt_strftime_format  ACTIVE  — fixtures and the Rust OracleRequest both spell it
                               `dt_strftime_format`; the handler read `dt_format`
                               and defaulted to "%Y-%m-%d". Every
                               series_dt_strftime case was evaluated with the
                               wrong format. Measured on FP-P2D-310: pinned
                               '2024/03/15 14:30' vs oracle '2024-03-15'; adding
                               the key reproduced the pinned value exactly, so
                               the FIXTURE was right and the ORACLE was wrong.
  rolling_window      LATENT  — handler read `window_size`, default 3. All six
                               rolling fixtures happen to use window 3, so they
                               were correct BY ACCIDENT. The first fixture with
                               any other window would have been silently
                               evaluated at 3.

A latent one is the more dangerous kind: it is green today and becomes a wrong
answer the moment someone adds a case, with nothing to point at.

This test is the guard. It is intentionally CONSERVATIVE — a key counts as read
if its quoted literal appears anywhere in the oracle source. That cannot produce
a false alarm (a key the oracle really does read always appears), which is what
makes it safe to run in CI. It will miss a key that is mentioned but not
actually consumed; catching that needs the differ, not this test.
"""
from __future__ import annotations

import json
import pathlib

import pytest


ORACLE_ROOT = pathlib.Path(__file__).resolve().parents[1]
ORACLE_SOURCE = ORACLE_ROOT / "pandas_oracle.py"
FIXTURE_ROOT = ORACLE_ROOT.parents[0] / "fixtures" / "packets"

# Structural fixture keys — packet identity, mode, and the recorded expectation.
# These are consumed by the Rust harness, never sent to the oracle as options.
STRUCTURAL_KEYS = {
    "packet_id",
    "case_id",
    "mode",
    "operation",
    "fixture_provenance",
    "oracle_source",
    "expected_series",
    "expected_frame",
    "expected_join",
    "expected_alignment",
    "expected_bool",
    "expected_positions",
    "expected_scalar",
    "expected_dtype",
    "expected_error_contains",
    "expected_error",
}

# Keys that are legitimately not oracle options, or are known gaps with a bead.
# Every entry needs a reason. This list must SHRINK; adding to it to make the
# test pass is the failure mode it exists to prevent.
KNOWN_UNREAD = {
    # Binary IO payloads on one generated fixture. The oracle cannot derive
    # these operations from a base64 blob, which is why that fixture is
    # replay-only rather than live-derivable.
    "excel_input_base64": "replay-only binary IO fixture, not live-derivable",
    "feather_input_base64": "replay-only binary IO fixture, not live-derivable",
    "ipc_stream_input_base64": "replay-only binary IO fixture, not live-derivable",
    "parquet_input_base64": "replay-only binary IO fixture, not live-derivable",
    "requirement_level": "fixture metadata, not an operation argument",
    # The oracle has NO handler for `dataframe_compare` at all — it answers
    # "unsupported operation: 'dataframe_compare'" — so this key is unread
    # because the whole operation is not live-derivable, not because an option
    # is being dropped. fp_p2d_418 is replay-only. Retiring it (with the reason
    # recorded) versus implementing the op is a call for 9s0c4.
    "compare_result_names": "oracle has no dataframe_compare handler; fixture is replay-only",
    # Genuine gap of the SAME class as dt_strftime_format, still unfixed:
    # fp_p2d_023_dataframe_constructor_list_like_copy_true pins copy=True and
    # the constructor handler never reads it. Left for its own before/after
    # evidence rather than a batch edit. Tracked by
    # br-frankenpandas-fixture-divergence-triage-9s0c4.
    "constructor_copy": "9s0c4: fp_p2d_023_..._copy_true pins copy=True; oracle ignores it",
}


def _fixture_keys() -> dict[str, list[str]]:
    keys: dict[str, list[str]] = {}
    for path in sorted(FIXTURE_ROOT.glob("*.json")):
        fixture = json.loads(path.read_text())
        for key in fixture:
            keys.setdefault(key, []).append(path.name)
    return keys


def test_fixture_corpus_is_present():
    """Guard the guard: an empty corpus would make every assertion below vacuous."""
    assert FIXTURE_ROOT.is_dir()
    assert len(list(FIXTURE_ROOT.glob("*.json"))) > 1000


def test_every_fixture_option_key_is_read_by_the_oracle():
    source = ORACLE_SOURCE.read_text()
    unread = {
        key: paths
        for key, paths in _fixture_keys().items()
        if key not in STRUCTURAL_KEYS
        and key not in KNOWN_UNREAD
        and f'"{key}"' not in source
    }
    assert not unread, (
        "fixture option keys the oracle never reads — it will silently use a "
        "default and evaluate a different operation than the fixture asked for:\n"
        + "\n".join(
            f"  {key}: {len(paths)} fixtures, e.g. {paths[0]}"
            for key, paths in sorted(unread.items())
        )
    )


@pytest.mark.parametrize(
    "key",
    [
        "dt_strftime_format",
        "rolling_window",
        "str_wrap_drop_whitespace",
        "corr_numeric_only",
        "na_action_ignore",
    ],
)
def test_previously_dropped_option_keys_stay_read(key: str):
    """Regression pins for every key fixed under this class.

    Asserted by literal so a rename or a revert fails here rather than silently
    resuming the default-fallback behavior — which is precisely how these went
    unnoticed in the first place.
    """
    assert f'"{key}"' in ORACLE_SOURCE.read_text()


@pytest.mark.parametrize("key,reason", sorted(KNOWN_UNREAD.items()))
def test_known_unread_entries_are_still_unread(key: str, reason: str):
    """The allowlist must not outlive the gap it documents.

    When someone fixes one of these in the oracle, this fails and forces the
    entry to be removed — so the list can only shrink, and cannot quietly become
    a place where real gaps are parked forever.
    """
    source = ORACLE_SOURCE.read_text()
    assert f'"{key}"' not in source, (
        f"{key} is now read by the oracle ({reason}). Remove it from "
        "KNOWN_UNREAD so the guard covers it."
    )
