#!/usr/bin/env python3
"""Attribution-gated fixture regeneration for br-frankenpandas-fixture-corpus-stale-vs-oracle-p6srr.

This tool CANNOT bulk-regenerate, by construction. The maintainer's ruling:

    "NO bulk regeneration. Approved only per-fixture where each moved value is
     attributable to a named already-decided divergence; regenerating wholesale
     would overwrite evidence with the output of the code under test. Fixtures
     whose operations the oracle no longer supports get RETIRED with the reason
     recorded per fixture, not regenerated. Any moved value you cannot attribute
     stays a failing fixture."

So `--apply` writes ONLY fixtures named in an attribution file, and refuses (exit
3) if any fixture moved without one.

COMPARISON IS DELEGATED, NOT REIMPLEMENTED
    `fixture_differ.compare_expected` is the repo's semantic comparator and is
    imported here rather than copied. That file was found to over-report 420
    defects, 238 of them its own -- absent `column_order` read as an empty-list
    claim, serde aliases `str`/`string` treated as distinct from `utf8`. Any
    tool that re-derives equality re-derives those bugs. One implementation,
    one place to fix.

    An earlier version of THIS script compared with a naive `!=` on parsed JSON
    and therefore had the same false-positive class. The ~17.5% "VALUE MOVED"
    figure it produced is RETRACTED; do not plan against it.

COVERAGE IS REPORTED, NEVER ASSUMED
    `compare_expected` only handles expected_series / expected_frame /
    expected_scalar / expected_bool. Fixtures also carry expected_alignment,
    expected_join, expected_positions and expected_dtype. Those get a normalized
    structural comparison and are COUNTED AND LABELLED SEPARATELY.

    This matters because the differ's worst bug was silent non-comparison: a bad
    default made its value loop iterate nothing, so a whole class of frames was
    never compared while the run still reported success. A key this tool cannot
    compare is reported as UNCOMPARED. It is never counted as agreement.
"""
from __future__ import annotations

import argparse
import collections
import concurrent.futures
import datetime as dt
import json
import subprocess
import sys
from pathlib import Path
from typing import Any

PROJECT_ROOT = Path(__file__).resolve().parent.parent
ORACLE_DIR = PROJECT_ROOT / "crates/fp-conformance/oracle"
ORACLE = ORACLE_DIR / "pandas_oracle.py"
FIXTURE_ROOT = PROJECT_ROOT / "crates/fp-conformance/fixtures/packets"
PROVENANCE_KEY = "fixture_provenance"

sys.path.insert(0, str(ORACLE_DIR))
try:
    from fixture_differ import compare_expected, normalize_kind  # type: ignore
except ImportError as exc:  # pragma: no cover
    print(f"cannot import fixture_differ (owned by the conformance lane): {exc}", file=sys.stderr)
    raise SystemExit(2) from exc

# Keys fixture_differ.compare_expected adjudicates semantically.
SEMANTIC_KEYS = ("expected_series", "expected_frame", "expected_scalar", "expected_bool")

# The key by which a fixture asserts the operation must FAIL.
ERROR_KEY = "expected_error_contains"
ERROR_EXPECTED_BUT_SUCCEEDED = "ERROR expected-but-succeeded"

# Where an oracle refusal came from; see `oracle_error_origin` in
# pandas_oracle.py. Only PANDAS supports an error-agreement attestation.
ORIGIN_KEY = "error_origin"
ORIGIN_PANDAS = "pandas"

# What a fixture's provenance stamp CLAIMS was verified.
#
# Absent  -> the default and strongest reading: "running this oracle script on
#            this input reproduced these expected VALUES".
# Present -> a weaker, explicitly-named claim. `ATTESTATION_ERROR_AGREEMENT`
#            means "running this oracle script on this input made PANDAS raise
#            too". It says nothing about the message: `expected_error_contains`
#            pins FrankenPandas's wording, checked by the Rust harness, while
#            the oracle surfaces pandas' own English, and the two were never
#            meant to match.
#
# Naming it in the artifact is the whole point. Restamping error fixtures under
# the bare key would silently widen what oracle_script_sha256 means across the
# corpus; this makes the weaker claim legible instead.
ATTESTATION_KEY = "oracle_attestation"
ATTESTATION_ERROR_AGREEMENT = "error_agreement"


def fixture_expects_error(fixture: dict[str, Any]) -> bool:
    """Does this fixture assert that the operation must fail?

    A bare `null` under the key is NOT such an assertion — generated fixtures
    carry every `expected_*` key with a null placeholder, and one of them was
    inflating the uncompared count. A missing value is encoded as
    `{"kind": "null", ...}`, never as bare JSON null, so this cannot swallow a
    real claim.
    """
    return fixture.get(ERROR_KEY) is not None


def differing_leaves(pinned: Any, live: Any, path: str = "") -> list[tuple[str, Any, Any]]:
    """Every differing leaf between two normalized trees, with its path.

    Walks BOTH sides and reports shape differences explicitly, because the
    fixture_differ bug this tool exists downstream of was a silent
    non-comparison: a default that made a loop iterate nothing while the run
    still reported success. A structure this cannot descend is reported as a
    difference at that node, never skipped.
    """
    if isinstance(pinned, dict) and isinstance(live, dict):
        out: list[tuple[str, Any, Any]] = []
        for key in sorted(set(pinned) | set(live)):
            if key not in pinned or key not in live:
                out.append((f"{path}.{key}", pinned.get(key, "<absent>"), live.get(key, "<absent>")))
            else:
                out.extend(differing_leaves(pinned[key], live[key], f"{path}.{key}"))
        return out
    if isinstance(pinned, list) and isinstance(live, list):
        if len(pinned) != len(live):
            return [(f"{path}[len]", len(pinned), len(live))]
        out = []
        for i, (p, l) in enumerate(zip(pinned, live)):
            out.extend(differing_leaves(p, l, f"{path}[{i}]"))
        return out
    if pinned != live:
        return [(path or ".", pinned, live)]
    return []


def ignored_by_adjudicator(leaf_path: str, pinned: Any, live: Any) -> bool:
    """True for leaves `fixture_differ.frame_equal` deliberately does not count.

    Mirrors ONE named rule rather than re-deriving equality: `frame_column_order`
    treats a missing or empty `column_order` as "this side recorded no ordering
    claim", and `frame_equal` then compares the column SET instead. Most fixtures
    store no `column_order` at all while the live oracle always emits one, so
    counting that leaf labelled 61 fixtures `SHAPE key-added-by-oracle` and put a
    non-difference in the example line as the exemplar of the move.

    That default-vs-populated comparison is the exact bug that produced 269 of
    the differ's 420 phantom rows. It is not repeated here.
    """
    if not leaf_path.endswith((".column_order", ".column_order[len]")):
        return False
    return any(side == "<absent>" or side == [] or side == 0 for side in (pinned, live))


def classify_move(pinned: Any, live: Any) -> tuple[list[str], str]:
    """Bucket a moved value into countable classes.

    Returns (classes, example). Classification is STRUCTURAL — it walks the
    trees rather than parsing a human-readable mismatch string — so the counts
    do not depend on message formatting.

    TWO RULES, both from the maintainer and both about not letting one
    attribution stand in for two mechanisms:

    1. DIRECTION IS PART OF THE CLASS. `null->na_n` and `na_n->null` may have
       entirely different explanations, so they are different classes. The same
       test is applied to every class here: `int64->float64` is not
       `float64->int64`, a key the oracle ADDED is not a key it DROPPED, and a
       sequence that grew is not one that shrank.

    2. A FIXTURE WITH SEVERAL MECHANISMS IS COUNTED IN EACH. Returning only the
       first class would file a fixture exhibiting both a dtype promotion and a
       null-marker change under one of them, and the other mechanism would
       vanish from the counts entirely. Callers therefore count (fixture, class)
       pairs, and the class totals may exceed the number of moved fixtures.
    """
    # Compare the kind-normalized trees so serde aliases never register, then
    # drop the leaves the ADJUDICATOR does not count as differences. Classifying
    # a leaf `compare_expected` deliberately ignores invents a mechanism out of
    # a non-difference, and an attribution pass would then "explain" fixtures
    # that never moved on that key.
    leaves = [
        leaf
        for leaf in differing_leaves(normalize_structural(pinned), normalize_structural(live))
        if not ignored_by_adjudicator(*leaf)
    ]
    if not leaves:
        return ["NORMALIZES_EQUAL"], ""

    path, p, l = leaves[0]
    example = f"{path}: {p!r} vs {l!r}"
    classes: set[str] = set()

    for leaf_path, lp, ll in leaves:
        # Dtype changes, directional: int64->float64 is not float64->int64.
        if leaf_path.endswith(".kind") and isinstance(lp, str) and isinstance(ll, str):
            classes.add(f"KIND {lp}->{ll}")
        # Null discriminator, directional: null->na_n is not na_n->null.
        elif (
            leaf_path.endswith(".value")
            and isinstance(lp, str)
            and isinstance(ll, str)
            and lp in NULL_MARKERS
            and ll in NULL_MARKERS
        ):
            classes.add(f"NULL_MARKER {lp}->{ll}")
        # Key presence, directional: a key the oracle ADDED is a different
        # mechanism from one it DROPPED.
        elif ll == "<absent>":
            classes.add("SHAPE key-dropped-by-oracle")
        elif lp == "<absent>":
            classes.add("SHAPE key-added-by-oracle")
        # Sequence length, directional for the same reason.
        elif leaf_path.endswith("[len]") and isinstance(lp, int) and isinstance(ll, int):
            classes.add("SHAPE longer" if ll > lp else "SHAPE shorter")
        else:
            classes.add("VALUE")

    return sorted(classes), example


NULL_MARKERS = {"null", "na_n", "nan"}


# "nan" and "na_n" decode to the SAME value (`scalar_from_json` maps both to
# float('nan')), so they must not read as different markers here.
MARKER_ALIASES = {"nan": "na_n"}


def canonical_marker(marker: str) -> str:
    return MARKER_ALIASES.get(marker, marker)


def input_null_markers(fixture: dict[str, Any]) -> set[str]:
    """Null markers the oracle was HANDED, from the fixture's inputs only.

    Deliberately excludes every `expected*` block: reading the pinned answer
    back in would make every null-marker move look like a round-trip loss,
    which is the exact false positive this discriminator exists to avoid.
    """
    markers: set[str] = set()

    def walk(node: Any) -> None:
        if isinstance(node, dict):
            if node.get("kind") == "null" and isinstance(node.get("value"), str):
                markers.add(canonical_marker(node["value"]))
            for value in node.values():
                walk(value)
        elif isinstance(node, list):
            for value in node:
                walk(value)

    for key, value in fixture.items():
        if key.startswith("expected") or key in (PROVENANCE_KEY, "retired"):
            continue
        walk(value)
    return markers


def roundtrip_implicated(fixture: dict[str, Any], classes: list[str]) -> list[str]:
    """Null-marker classes whose PINNED marker was already in the fixture's input.

    THE DISCRIMINATOR THIS TOOL WAS MISSING, and the reason the 85-fixture
    `NULL_MARKER null->na_n` class must not take one attribution:

      - marker present in the input -> the oracle could not return a marker it
        was GIVEN. On an identity-shaped op that is a read/write asymmetry in
        the oracle, provable without pandas at all, and the pinned value is
        right by construction.
      - marker absent from the input -> the missing value was INTRODUCED by the
        operation (alignment, outer merge, reindex). Nothing here says who is
        right; that one still needs a live-pandas probe of its own.

    OBSERVED, which is what licenses adding a check at all: fp_p2c_010_series_
    head_with_nulls_hardened pins `{"kind":"null","value":"null"}` at values[1]
    of BOTH its input and its expectation -- `head(3)` does not touch that
    element -- and the oracle emits `na_n`. `series_dtype_for_payload_values`
    maps int64+null to nullable `Int64` (which round-trips as pd.NA) but
    float64+null to plain `float64`, where None collapses to nan and `na_n`
    becomes the only marker the oracle can write. Same read/write asymmetry
    shape as the bool-label bug fixed in 6bqfr, one dtype family over.

    Direction is load-bearing: a fixture whose input carries only `na_n` does
    NOT explain a pinned `null`, so the PINNED side of the class is what gets
    looked up, never merely "some marker appeared in the input".
    """
    if not classes:
        return []
    available = input_null_markers(fixture)
    implicated: list[str] = []
    for klass in classes:
        if not klass.startswith("NULL_MARKER "):
            continue
        direction = klass[len("NULL_MARKER "):]
        pinned, _, live = direction.partition("->")
        if not live:
            continue
        if canonical_marker(pinned) in available:
            implicated.append(klass)
    return sorted(implicated)


def normalize_structural(node: Any) -> Any:
    """Apply the differ's kind-alias normalization recursively.

    Not a semantic comparison — just enough to stop `str`/`string` vs `utf8`
    from reading as a real difference in the keys compare_expected does not
    cover. Anything this decides is labelled STRUCTURAL in the report.
    """
    if isinstance(node, dict):
        if "kind" in node:
            node = normalize_kind(node)
        return {k: normalize_structural(v) for k, v in node.items()}
    if isinstance(node, list):
        return [normalize_structural(v) for v in node]
    return node


def run_oracle(
    fixture: dict[str, Any],
    legacy_root: str,
    timeout: int,
    oracle_path: Path | None = None,
) -> dict[str, Any]:
    proc = subprocess.run(
        [sys.executable, str(oracle_path or ORACLE), "--legacy-root", legacy_root,
         "--allow-system-pandas-fallback"],
        input=json.dumps(fixture).encode(),
        capture_output=True, timeout=timeout, check=False,
    )
    if not proc.stdout:
        raise RuntimeError(
            f"oracle produced no stdout (exit {proc.returncode}): "
            f"{proc.stderr.decode(errors='replace')[:300]}"
        )
    return json.loads(proc.stdout)


def classify(fixture: dict[str, Any], response: dict[str, Any]) -> dict[str, Any]:
    """Compare one fixture against a live oracle response.

    Returns a verdict dict. `moved` lists keys that genuinely differ; `how`
    records whether each key was adjudicated SEMANTIC or STRUCTURAL; `uncompared`
    lists keys present in the fixture that neither path could judge.
    """
    # A bare-null expected key is a placeholder, not a claim. Counting them made
    # generated fixtures contribute phantom uncompared keys.
    present = [k for k in fixture if k.startswith("expected") and fixture[k] is not None]
    moved: list[str] = []
    how: dict[str, str] = {}
    uncompared: list[str] = []
    detail: dict[str, str] = {}

    semantic_present = [k for k in present if k in SEMANTIC_KEYS]
    if semantic_present:
        agree, why = compare_expected(fixture, response)
        for key in semantic_present:
            how[key] = "SEMANTIC"
        if not agree:
            moved.extend(semantic_present)
            detail["semantic"] = why

    for key in present:
        if key in SEMANTIC_KEYS:
            continue
        if key == ERROR_KEY:
            # `classify` only runs when the oracle SUCCEEDED, so a fixture that
            # asserts failure has been contradicted. This used to be reported as
            # an UNCOMPARED key, which let 11 fixtures saying "this must fail"
            # be counted as agreement while pandas happily returned a value —
            # the silent-non-comparison-as-success bug, in this tool.
            moved.append(key)
            how[key] = "SEMANTIC"
            detail["expected_error"] = (
                f"fixture requires failure containing {fixture[key]!r}; the oracle succeeded"
            )
            continue
        if key not in response:
            uncompared.append(key)
            continue
        how[key] = "STRUCTURAL"
        if normalize_structural(response[key]) != normalize_structural(fixture[key]):
            moved.append(key)

    provenance_stale = response.get(PROVENANCE_KEY) != fixture.get(PROVENANCE_KEY)
    return {
        "moved": moved,
        "how": how,
        "uncompared": uncompared,
        "detail": detail,
        "provenance_stale": provenance_stale,
    }


PROVENANCE_FICTION = "PROVENANCE_FICTION"
GENUINELY_STALE = "GENUINELY_STALE"
BOTH_MOVED = "BOTH_MOVED"
STAMP_IMPOSSIBLE = "STAMP_IMPOSSIBLE"
STAMP_ERRORED = "STAMP_ERRORED"


def named_oracle_verdict(named_error: str) -> str:
    """Verdict when the named oracle produced no answer at all.

    Reported, never silently dropped. This tool's whole premise is that a
    comparison it cannot make is named rather than skipped — the differ's worst
    bug was a silent non-comparison that still reported success — and "the named
    oracle refuses this fixture's operation" is a LOUDER provenance failure than
    a value that merely moved, not a gap in coverage.

    OBSERVED: 55 of the 56 fixtures the generation-era oracle could not answer
    failed with `unsupported operation`. A fixture stamped with an oracle that
    does not implement its operation cannot have been generated by that oracle,
    so the stamp is impossible on its face — no value comparison required.
    """
    return STAMP_IMPOSSIBLE if "unsupported operation" in named_error else STAMP_ERRORED


def provenance_verdict(named_matches_fixture: bool, named_matches_current: bool) -> str:
    """Did the fixture's OWN named oracle produce the values it pins?

    This is the question p6srr assumes the answer to. The bead is titled "the
    corpus is stale against its oracle", which presumes the pinned values were
    once correct output that the oracle has since moved away from. That
    presumption is testable, and for at least one class it is FALSE.

    OBSERVED: `fp_p2d_028_dataframe_concat_axis1_basic_strict` pins int64 values
    with `"null"` markers. Its `oracle_script_sha256` is `f38b2fca…`, which is
    exactly the sha of `pandas_oracle.py` at 9aa1ed6fe (2026-04-22) — the stamp
    names a real, identifiable oracle. Run that exact oracle on that exact
    fixture today, on the pinned pandas 2.2.3, and it emits float64 + `na_n`,
    identical to the CURRENT oracle. The named oracle could not have produced
    the pinned expectation.

    So the three outcomes are genuinely different work:

      GENUINELY_STALE     the named oracle DID produce these values and today's
                          does not -> the oracle changed; regeneration is the
                          right remedy once the change is understood.
      PROVENANCE_FICTION  neither oracle produces them -> the values were
                          authored, not generated, and the provenance stamp is
                          not evidence of anything. Regenerating DELETES a
                          hand-authored parity target. DISC-011 names
                          FP-P2D-028 as exactly such a WILL-FIX target.
      BOTH_MOVED          the named oracle produces something that is neither
                          the pinned value nor today's answer -> two changes
                          stacked; needs its own look.

    Collapsing these into one "stale" total is what would license a bulk
    regeneration over fixtures that were never generated in the first place.
    """
    if named_matches_fixture:
        return GENUINELY_STALE
    if named_matches_current:
        return PROVENANCE_FICTION
    return BOTH_MOVED


def rebuild(fixture: dict[str, Any], response: dict[str, Any]) -> dict[str, Any]:
    """Fixture with regenerated expected values + fresh provenance.

    Only keys the fixture ALREADY HAS are written: the oracle response carries
    every expected_* block, mostly null, and copying them wholesale would bloat
    the corpus with nulls and destroy the diff's meaning.
    """
    updated = json.loads(json.dumps(fixture))
    for key in fixture:
        if key.startswith("expected") and key in response:
            updated[key] = response[key]
    if PROVENANCE_KEY in response:
        updated[PROVENANCE_KEY] = response[PROVENANCE_KEY]
    return updated


def corpus_wide_apply_refused(
    *, apply: bool, restamp_agreeing: bool, glob: str, limit: int
) -> bool:
    """Is this a corpus-wide `--apply` that must be refused?

    The ban exists to stop a bulk REGENERATION sweeping expected values in from
    the oracle — that is how a real divergence gets laundered into the corpus as
    pinned truth.

    `--restamp-agreeing` is exempt, because it is a different operation wearing
    the same flag. It reads no expected value at all (`restamp` copies only
    `fixture_provenance`), it fires only where `restampable` proved the current
    oracle reproduced everything the fixture pins, and with no `--attributions`
    the allowlist is empty so zero MOVED fixtures can be written. That is the
    same carve-out the `--attributions` check in `main` already makes, for the
    same reason.

    Without the exemption the restamp mode is unrunnable at the only scale it
    exists for: br-frankenpandas-fixture-corpus-stale-vs-oracle-p6srr is 1144
    agreeing fixtures pinning a stale script hash, and `restamp`'s own docstring
    calls itself "the honest remedy for the bulk of p6srr". Batching it behind
    `--limit 400` to satisfy the guard's letter would be evading the guard, which
    is worse than narrowing it in the open.

    Lifted out of `main` so the rule is assertable without running the corpus —
    the argument-level test for it previously had to invoke the whole sweep.
    """
    return apply and not restamp_agreeing and glob == "*.json" and not limit


def restampable(verdict: dict[str, Any]) -> bool:
    """May this fixture's provenance stamp be refreshed?

    ONLY when the current oracle reproduced everything the fixture pins:

      * nothing moved, and
      * nothing went UNCOMPARED — a key the adjudicator could not judge means
        the run did not verify that claim, and
      * at least one key was actually compared.

    The last two conditions are the point. `oracle_script_sha256` asserts "this
    oracle produced these values"; stamping a fixture whose keys were skipped
    would put that assertion on a value nothing checked — the
    silent-non-comparison-as-success bug, committed into the corpus as
    provenance. A fixture with zero comparable keys is therefore NOT
    restampable, however green it looks.
    """
    return (
        not verdict["moved"]
        and not verdict["uncompared"]
        and bool(verdict["how"])
    )


def restamp_text(raw: str, old: dict[str, str], new: dict[str, str]) -> str:
    """Rewrite the provenance values IN PLACE in the fixture's own source text.

    A `json.dumps(..., indent=2)` round-trip would reformat every fixture it
    touches: the corpus writes each scalar on one line (`{ "kind": "int64",
    "value": 0 }`) and the dumper explodes that to five. Restamping is a
    3-line-per-file change applied to ~1000 files, so a reformat would bury the
    only thing that actually changed under ~60 lines of noise per fixture and
    make the diff unreviewable — precisely when reviewability is the point.

    Substitution is confined to the `fixture_provenance` block (located by brace
    matching) so a value like the pandas version cannot be rewritten where it
    happens to appear elsewhere in the file. Each key is replaced exactly once.
    Returns the text unchanged if the block cannot be located, and the caller's
    verification (parse + compare) is what proves the result correct.
    """
    marker = f'"{PROVENANCE_KEY}"'
    start = raw.find(marker)
    if start == -1:
        return raw
    open_brace = raw.find("{", start)
    if open_brace == -1:
        return raw
    depth = 0
    end = -1
    for idx in range(open_brace, len(raw)):
        if raw[idx] == "{":
            depth += 1
        elif raw[idx] == "}":
            depth -= 1
            if depth == 0:
                end = idx + 1
                break
    if end == -1:
        return raw

    block = raw[open_brace:end]
    inserts: list[tuple[str, Any]] = []
    for key, new_value in new.items():
        old_value = old.get(key)
        if old_value is None:
            # A key the fixture does not carry yet (the attestation). Collected
            # and appended below rather than skipped, so a weaker claim can be
            # written EXPLICITLY instead of being implied by silence.
            if key not in old:
                inserts.append((key, new_value))
            continue
        if old_value == new_value:
            continue
        needle = f'"{key}": {json.dumps(old_value)}'
        replacement = f'"{key}": {json.dumps(new_value)}'
        if needle in block:
            block = block.replace(needle, replacement, 1)

    for key, value in inserts:
        # Append inside the block, after the last existing entry. Indentation is
        # copied from the final line so the corpus keeps its own formatting.
        closing = block.rfind("}")
        head, tail = block[:closing], block[closing:]
        # The whitespace that indented the CLOSING BRACE lives at the end of
        # `head`, and `rstrip()` below removes it. Capture it first, or the `}`
        # is re-emitted at column 0 — which is still valid JSON, so nothing
        # fails, it just silently deforms every fixture this touches. That is
        # exactly the noise `restamp_text` exists to avoid: it landed in 71 files
        # before this was caught by reading the diff rather than the exit code.
        closing_indent = head[head.rfind("\n") + 1 :] if "\n" in head else ""
        stripped = head.rstrip()
        indent = ""
        last_newline = stripped.rfind("\n")
        if last_newline != -1:
            indent = stripped[last_newline + 1 :][
                : len(stripped[last_newline + 1 :])
                - len(stripped[last_newline + 1 :].lstrip())
            ]
        entry = f'{stripped},\n{indent}"{key}": {json.dumps(value)}\n{closing_indent}'
        block = entry + tail
    return raw[:open_brace] + block + raw[end:]


def provenance_write_is_faithful(
    before: dict[str, Any],
    after: dict[str, Any],
    oracle_provenance: dict[str, Any],
    added: dict[str, Any],
) -> bool:
    """Did the provenance rewrite say exactly what it was supposed to say?

    Replaces a whole-dict `after == oracle_provenance` equality, which was
    strictly WRONG for any fixture carrying richer provenance than the oracle
    emits. `fp_generated_tn6qb2_...` records generation_command, input_matrix and
    intentional_divergence_notes; the oracle's stamp has three keys, so whole-dict
    equality could only be satisfied by DESTROYING that evidence, and the writer
    (correctly) refused rather than flatten it.

    The freshness gate never required flattening: check_fixture_freshness.sh
    reads pandas_version / oracle_script_sha256 / generated_at through
    `provenance.get()` and ignores every other key. A provenance SUPERSET is
    already gate-tolerable — the limitation was here, in the verifier.

    This is also STRICTER than the equality it replaces, because it additionally
    proves nothing was dropped:
      * every oracle-emitted key now carries the oracle's value
      * every other pre-existing key survives byte-identical
      * nothing appeared except the keys we deliberately added
    """
    for key, value in oracle_provenance.items():
        if after.get(key) != value:
            return False

    preserved = set(before) - set(oracle_provenance)
    for key in preserved:
        if after.get(key) != before[key]:
            return False

    for key, value in added.items():
        if after.get(key) != value:
            return False

    return set(after) == set(before) | set(oracle_provenance) | set(added)


def write_restamp(
    path: Path,
    fixture: dict[str, Any],
    response: dict[str, Any],
    attestation: str | None = None,
) -> bool:
    """Refresh one fixture's provenance IN PLACE. True if written.

    Text-level so the corpus keeps its compact formatting and the diff stays a
    few lines per fixture (a json.dumps round-trip would reformat ~1000 files).
    VERIFIED BEFORE IT LANDS: the result must parse, must satisfy
    `provenance_write_is_faithful`, and must leave every non-provenance key
    equal under the parser. A rewrite that moved an expected value would be
    regeneration, so it is refused rather than written.
    """
    raw = path.read_text(encoding="utf-8")
    before = fixture.get(PROVENANCE_KEY) or {}
    oracle_provenance = response.get(PROVENANCE_KEY) or {}
    added = {ATTESTATION_KEY: attestation} if attestation else {}

    updated = restamp_text(raw, before, {**oracle_provenance, **added})
    try:
        reparsed = json.loads(updated)
    except json.JSONDecodeError:
        return False

    after = reparsed.get(PROVENANCE_KEY)
    if not isinstance(after, dict):
        return False
    if not provenance_write_is_faithful(before, after, oracle_provenance, added):
        return False
    if {k: v for k, v in reparsed.items() if k != PROVENANCE_KEY} != {
        k: v for k, v in fixture.items() if k != PROVENANCE_KEY
    }:
        return False

    path.write_text(updated, encoding="utf-8")
    return True


def restamp(fixture: dict[str, Any], response: dict[str, Any]) -> dict[str, Any]:
    """Fixture with a refreshed provenance stamp and IDENTICAL expected values.

    This is the honest remedy for the bulk of p6srr. The corpus is not mostly
    wrong — for most fixtures the current oracle reproduces the pinned values
    exactly and only the stamp names an older script. Rewriting the stamp there
    makes it TRUE; it does not relax the freshness gate, and it is not
    regeneration, because no expected value is read from the oracle at all.

    Only `fixture_provenance` is touched. Callers must gate on `restampable`.
    """
    updated = json.loads(json.dumps(fixture))
    if PROVENANCE_KEY in response:
        updated[PROVENANCE_KEY] = response[PROVENANCE_KEY]
    return updated


def retire(fixture: dict[str, Any], reason: str) -> dict[str, Any]:
    """Mark a fixture retired IN THE FIXTURE, with its reason.

    Expected values are left exactly as they are: a retired fixture keeps its
    evidence, it just stops claiming to be regenerable.
    """
    updated = json.loads(json.dumps(fixture))
    updated["retired"] = {
        "reason": reason,
        "retired_at": dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "bead": "br-frankenpandas-fixture-corpus-stale-vs-oracle-p6srr",
    }
    return updated


def load_attributions(path: Path | None) -> dict[str, dict[str, dict[str, str]]]:
    if path is None:
        return {"attributions": {}, "retirements": {}}
    data = json.loads(path.read_text(encoding="utf-8"))
    for section in ("attributions", "retirements"):
        data.setdefault(section, {})
    for name, entry in data["attributions"].items():
        if not entry.get("divergence"):
            raise SystemExit(
                f"attribution for {name} has no 'divergence' — every approved "
                f"regeneration must name an already-decided divergence"
            )
    for name, entry in data["retirements"].items():
        if not entry.get("reason"):
            raise SystemExit(f"retirement for {name} has no 'reason'")
    return data


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--legacy-root", default="/nonexistent")
    parser.add_argument("--attributions", type=Path, help="JSON allowlist; required for --apply")
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--restamp-agreeing", action="store_true",
                        help="Refresh fixture_provenance ONLY on fixtures whose pinned values "
                             "the current oracle reproduces exactly (nothing moved, nothing "
                             "uncompared). Expected values are never read from the oracle in "
                             "this mode, so it is not regeneration and needs no attributions. "
                             "Without --apply it reports how many WOULD be restamped.")
    parser.add_argument("--glob", default="*.json")
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--jobs", type=int, default=1,
                        help="concurrent oracle invocations; report stays deterministic")
    parser.add_argument("--list-limit", type=int, default=20,
                        help="how many individual moves to print (class COUNTS are always complete)")
    parser.add_argument("--provenance-oracle", type=Path,
                        help="Path to the oracle a fixture's oracle_script_sha256 NAMES "
                             "(extract it with `git show <commit>:...pandas_oracle.py`). "
                             "Moved fixtures are additionally run through it to separate "
                             "genuine staleness from a provenance stamp that never "
                             "produced the pinned values.")
    parser.add_argument("--report-json", type=Path,
                        help="write the COMPLETE machine-readable report: every move with its "
                             "class, every error, no truncation. Required input for an "
                             "attribution pass driven by counts rather than by a visible slice.")
    args = parser.parse_args()

    if corpus_wide_apply_refused(
        apply=args.apply,
        restamp_agreeing=args.restamp_agreeing,
        glob=args.glob,
        limit=args.limit,
    ):
        print(
            "--apply requires an explicit --glob or --limit: corpus-wide regeneration is forbidden",
            file=sys.stderr,
        )
        return 2

    if args.apply and not args.restamp_agreeing and args.report_json is None:
        print(
            "--apply regeneration requires --report-json for complete attribution review",
            file=sys.stderr,
        )
        return 2

    # --restamp-agreeing rewrites no expected value, so it does not need an
    # attribution allowlist. With no --attributions the allowlist loads empty,
    # which means zero MOVED fixtures can be written — the bulk-regeneration ban
    # still holds exactly as before.
    if args.apply and args.attributions is None and not args.restamp_agreeing:
        print("--apply requires --attributions: bulk regeneration is forbidden "
              "(p6srr / maintainer ruling)", file=sys.stderr)
        return 2
    allow = load_attributions(args.attributions)

    fixtures = sorted(FIXTURE_ROOT.glob(args.glob))
    if args.limit:
        fixtures = fixtures[: args.limit]
    if args.apply and not fixtures:
        print("--apply matched no fixtures; refusing a no-op regeneration", file=sys.stderr)
        return 2

    def examine(path: Path):
        fixture = json.loads(path.read_text(encoding="utf-8"))
        try:
            response = run_oracle(fixture, args.legacy_root, args.timeout)
        except Exception as exc:  # noqa: BLE001
            return path, fixture, None, str(exc)[:160]
        if response.get("error"):
            # The RESPONSE is carried through even on the error path. It holds a
            # fresh fixture_provenance (error_response builds one) and the
            # error_origin that decides whether an expected-error fixture can be
            # attested at all. Returning None here is what kept the 85
            # expected-error agreements permanently unstampable.
            return path, fixture, response, f"oracle error: {response['error']}"
        return path, fixture, response, None

    def named_oracle_response(
        fixture: dict[str, Any],
    ) -> tuple[dict[str, Any] | None, str]:
        """(response, why-it-could-not-answer) from the oracle this fixture NAMES."""
        if args.provenance_oracle is None:
            return None, ""
        try:
            response = run_oracle(
                fixture, args.legacy_root, args.timeout, args.provenance_oracle
            )
        except Exception as exc:  # noqa: BLE001 - a refusal IS an answer about provenance
            return None, str(exc)[:160]
        error = response.get("error")
        return (None, str(error)) if error else (response, "")

    if args.jobs > 1:
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
            outcomes = list(pool.map(examine, fixtures))
    else:
        outcomes = [examine(p) for p in fixtures]

    agreed = prov_only = 0
    restampable_count = restamped = 0
    restamp_refused: list[str] = []
    moved_attributed: list[tuple[str, str]] = []
    moved_unattributed: list[tuple[str, list[str], str, str, str, list[str]]] = []
    move_classes: collections.Counter[str] = collections.Counter()
    roundtrip_classes: collections.Counter[str] = collections.Counter()
    provenance_verdicts: collections.Counter[str] = collections.Counter()
    retired: list[tuple[str, str]] = []
    unsupported: list[tuple[str, str]] = []
    other_errors: list[tuple[str, str]] = []
    expected_error_agreements: list[tuple[str, str, str | None]] = []
    error_agreement_pandas = 0
    error_agreement_stamped = 0
    error_agreement_not_pandas: list[tuple[str, str]] = []
    uncompared_keys: collections.Counter[str] = collections.Counter()
    how_counts: collections.Counter[str] = collections.Counter()

    for path, fixture, response, error in outcomes:
        name = path.name
        if error is not None or response is None:
            msg = error or "unknown"
            if "unsupported operation" in msg:
                unsupported.append((name, msg))
                entry = allow["retirements"].get(name)
                if entry and args.apply:
                    path.write_text(
                        json.dumps(retire(fixture, entry["reason"]), indent=2) + "\n",
                        encoding="utf-8")
                    retired.append((name, entry["reason"]))
            elif fixture_expects_error(fixture):
                # The fixture asserts this operation fails and the oracle failed.
                # Both sides agree it is an error, so this is NOT an oracle defect
                # to triage. The MESSAGE TEXT is deliberately not compared:
                # `expected_error_contains` pins FrankenPandas's wording, which the
                # Rust harness checks; the oracle raises pandas' own English and
                # the two were never meant to match ('out of bounds' vs pandas'
                # 'out-of-bounds' is the shape of that trap).
                #
                # BUT "the oracle failed" is not one fact. Only a refusal that
                # came from PANDAS says anything about pandas; a refusal from the
                # adapter's own argument validation means pandas was never
                # invoked, and stamping it would attest agreement on a question
                # nobody asked. Split by origin and stamp only the first kind.
                origin = (response or {}).get(ORIGIN_KEY)
                expected_error_agreements.append((name, msg, origin))
                if origin == ORIGIN_PANDAS:
                    error_agreement_pandas += 1
                    if args.restamp_agreeing and args.apply and response:
                        if write_restamp(
                            path, fixture, response, ATTESTATION_ERROR_AGREEMENT
                        ):
                            error_agreement_stamped += 1
                        else:
                            restamp_refused.append(name)
                else:
                    error_agreement_not_pandas.append((name, origin or "unknown"))
            else:
                other_errors.append((name, msg))
            continue

        verdict = classify(fixture, response)
        for key in verdict["uncompared"]:
            uncompared_keys[key] += 1
        for key, kind in verdict["how"].items():
            how_counts[kind] += 1

        if verdict["moved"]:
            entry = allow["attributions"].get(name)
            if entry:
                moved_attributed.append((name, entry["divergence"]))
                if args.apply:
                    path.write_text(
                        json.dumps(rebuild(fixture, response), indent=2) + "\n",
                        encoding="utf-8")
            else:
                # Classify structurally so the attribution pass can work by
                # CLASS rather than by eyeballing whatever sorts first.
                classes: list[str] = []
                example = ""
                for key in verdict["moved"]:
                    if key in fixture and key in response:
                        found, ex = classify_move(fixture[key], response[key])
                        classes.extend(found)
                        example = example or ex
                if ERROR_KEY in verdict["moved"]:
                    classes.append(ERROR_EXPECTED_BUT_SUCCEEDED)
                classes = sorted(set(classes)) or ["UNCLASSIFIED"]
                # Split each null-marker class by whether the pinned marker came
                # from this fixture's own input, so one attribution cannot cover
                # two mechanisms with opposite verdicts.
                implicated = roundtrip_implicated(fixture, classes)
                # Ask the fixture's OWN named oracle whether it ever produced
                # these values. "Stale" and "never generated" need opposite
                # remedies and must not share a total.
                prov = ""
                named, named_error = named_oracle_response(fixture)
                if named is None and named_error:
                    prov = named_oracle_verdict(named_error)
                    provenance_verdicts[prov] += 1
                elif named is not None:
                    # Compare the named oracle against BOTH sides using only the
                    # keys this fixture actually pins. A raw oracle response
                    # carries every expected_* block, mostly null, and comparing
                    # those would manufacture differences.
                    current_as_pinned = {
                        key: response[key]
                        for key in fixture
                        if key.startswith("expected") and key in response
                    }
                    prov = provenance_verdict(
                        not classify(fixture, named)["moved"],
                        not classify(current_as_pinned, named)["moved"],
                    )
                    provenance_verdicts[prov] += 1
                moved_unattributed.append(
                    (name, verdict["moved"], verdict["detail"].get("semantic", "")[:110],
                     classes, example, implicated, prov))
                # A fixture exhibiting several mechanisms is counted in EACH, so
                # no mechanism can hide behind another. Totals therefore exceed
                # the moved-fixture count; the report says so.
                for klass in classes:
                    move_classes[klass] += 1
                for klass in implicated:
                    roundtrip_classes[klass] += 1
        elif verdict["provenance_stale"]:
            prov_only += 1
            if restampable(verdict):
                restampable_count += 1
                if args.restamp_agreeing and args.apply:
                    if write_restamp(path, fixture, response):
                        restamped += 1
                    else:
                        restamp_refused.append(name)
        else:
            agreed += 1

    print(f"fixtures examined            : {len(fixtures)}")
    print(f"  agree, fully current       : {agreed}")
    print(f"  agree, provenance-only     : {prov_only}")
    print(f"    of those, RESTAMPABLE    : {restampable_count}"
          f"   (nothing moved, nothing uncompared)")
    print(f"    RESTAMPED this run       : {restamped}"
          f"{'' if args.restamp_agreeing and args.apply else '   (needs --restamp-agreeing --apply)'}")
    if restamp_refused:
        # Never silent. A restampable fixture the writer declined is a fixture
        # whose stamp stays stale, and an unreported one would read as covered.
        print(f"    RESTAMP REFUSED          : {len(restamp_refused)}"
              "   <-- richer provenance than the oracle emits; flattening it "
              "would DROP keys (generation_command, intentional_divergence_notes)")
        for name in restamp_refused:
            print(f"      {name}")
    print(f"  MOVED, attributed          : {len(moved_attributed)}")
    print(f"  MOVED, UNATTRIBUTED        : {len(moved_unattributed)}   <-- stay failing fixtures")
    print(f"  oracle: unsupported op     : {len(unsupported)}   <-- retire candidates")
    print(f"  expected-error, BOTH failed: {len(expected_error_agreements)}   "
          "<-- legitimate expected-error fixtures, NOT defects")
    print(f"    of those, PANDAS raised  : {error_agreement_pandas}"
          "   (attestable as error-agreement)")
    print(f"    ERROR-AGREEMENT STAMPED  : {error_agreement_stamped}"
          f"{'' if args.restamp_agreeing and args.apply else '   (needs --restamp-agreeing --apply)'}")
    if error_agreement_not_pandas:
        # Never a silent remainder. These are NOT stampable and the reason is
        # not "we ran out of time" — pandas was never invoked, so there is no
        # agreement to attest. Several look like adapter coverage gaps.
        by_origin: collections.Counter[str] = collections.Counter(
            origin for _, origin in error_agreement_not_pandas
        )
        print(f"    NOT attestable           : {len(error_agreement_not_pandas)}"
              "   <-- pandas NEVER INVOKED; adapter refused first")
        for origin, count in by_origin.most_common():
            print(f"      origin={origin}: {count}")
    print(f"  oracle: other errors       : {len(other_errors)}   <-- untriaged")
    print(f"\ncomparison coverage          : "
          f"{how_counts['SEMANTIC']} semantic, {how_counts['STRUCTURAL']} structural")
    if uncompared_keys:
        print("  UNCOMPARED KEYS (never counted as agreement):")
        for key, count in uncompared_keys.most_common():
            print(f"    {key}: {count}")
    if moved_unattributed:
        # COUNTS FIRST, and over every move — not a slice. A truncated listing
        # silently over-weights whatever sorts first, which is exactly how an
        # attribution pass ends up mis-sized.
        total_labels = sum(move_classes.values())
        print(
            f"\nMOVE CLASSES over all {len(moved_unattributed)} moved fixtures, largest first."
        )
        print(
            "  Direction is part of the class (a->b is not b->a), and a fixture with several\n"
            "  mechanisms is counted in EACH, so these total "
            f"{total_labels} labels across {len(moved_unattributed)} fixtures."
        )
        for klass, count in move_classes.most_common():
            share = 100.0 * count / len(moved_unattributed)
            roundtrip = roundtrip_classes.get(klass, 0)
            suffix = ""
            if klass.startswith("NULL_MARKER "):
                suffix = (f"   [{roundtrip} round-trip, "
                          f"{count - roundtrip} op-introduced]")
            print(f"  {count:5d}  {share:5.1f}% of fixtures  {klass}{suffix}")
        if roundtrip_classes:
            print(
                "\n  ROUND-TRIP means the pinned marker was in the fixture's own INPUT and the\n"
                "  oracle could not return it — an oracle read/write asymmetry, provable without\n"
                "  pandas. OP-INTRODUCED means the operation created the missing value, which is\n"
                "  a separate question and still needs its own live-pandas probe. The two must\n"
                "  not share one attribution."
            )
        if provenance_verdicts:
            moved_n = len(moved_unattributed)
            stale_n = provenance_verdicts[GENUINELY_STALE]
            not_drift = (provenance_verdicts[PROVENANCE_FICTION]
                         + provenance_verdicts[STAMP_IMPOSSIBLE])
            # State the partition against the SAME denominator the headline
            # number uses. "163 moved fixtures" is what gets carried forward, so
            # every verdict is reported as a share of it and the shares are
            # asserted to account for all of it.
            assert sum(provenance_verdicts.values()) == moved_n, (
                "provenance verdicts must partition the moved set"
            )
            print(
                f"\nDID THE FIXTURE'S OWN NAMED ORACLE EVER PRODUCE ITS PINNED VALUES?\n"
                f"  Of {moved_n} moved fixtures, {stale_n} "
                f"({100.0 * stale_n / moved_n:.1f}%) are stale in the sense "
                f"'the oracle drifted from\n  values it once produced'. "
                f"{not_drift} ({100.0 * not_drift / moved_n:.1f}%) are PROVABLY not that.\n"
                f"  {provenance_verdicts[GENUINELY_STALE]:5d}  {GENUINELY_STALE}"
                "     the named oracle DID; today's does not. Regeneration is the remedy.\n"
                f"  {provenance_verdicts[PROVENANCE_FICTION]:5d}  {PROVENANCE_FICTION}"
                "  neither oracle does. The values were AUTHORED, not generated;\n"
                "                             the stamp is not evidence. Regenerating deletes a target.\n"
                f"  {provenance_verdicts[BOTH_MOVED]:5d}  {BOTH_MOVED}"
                "        the named oracle produces a third answer. Two stacked changes.\n"
                f"  {provenance_verdicts[STAMP_IMPOSSIBLE]:5d}  {STAMP_IMPOSSIBLE}"
                "   the named oracle does not implement this fixture's operation,\n"
                "                             so it cannot have generated it. No comparison needed.\n"
                f"  {provenance_verdicts[STAMP_ERRORED]:5d}  {STAMP_ERRORED}"
                "      the named oracle failed for some other reason. Reported, not dropped.\n"
                f"  {'-' * 5}\n"
                f"  {moved_n:5d}  total — the verdicts partition the moved set exactly."
            )
        if args.list_limit:
            print(f"\nUNATTRIBUTED MOVES (first {args.list_limit} of "
                  f"{len(moved_unattributed)}; use --report-json for all):")
            for name, keys, why, classes, example, implicated, prov in (
                moved_unattributed[: args.list_limit]
            ):
                mark = " ROUND-TRIP" if implicated else ""
                mark += f" {prov}" if prov else ""
                print(f"  [{'+'.join(classes)}]{mark} {name}: {', '.join(keys)}  {why or example}")
    if unsupported:
        print("\nUNSUPPORTED OPERATION (first 10):")
        for name, msg in unsupported[:10]:
            print(f"  {name}: {msg}")
    if args.report_json:
        args.report_json.parent.mkdir(parents=True, exist_ok=True)
        args.report_json.write_text(
            json.dumps(
                {
                    "fixtures_examined": len(fixtures),
                    "agree_current": agreed,
                    "agree_provenance_only": prov_only,
                    "restampable": restampable_count,
                    "restamped": restamped,
                    "restamp_refused": restamp_refused,
                    "move_classes": dict(move_classes),
                    "roundtrip_classes": dict(roundtrip_classes),
                    "provenance_verdicts": dict(provenance_verdicts),
                    "moved_unattributed": [
                        {"fixture": n, "keys": k, "why": w, "classes": c, "example": e,
                         "roundtrip_implicated": r, "provenance": p}
                        for n, k, w, c, e, r, p in moved_unattributed
                    ],
                    "moved_attributed": [
                        {"fixture": n, "divergence": d} for n, d in moved_attributed
                    ],
                    "unsupported_operation": [
                        {"fixture": n, "error": m} for n, m in unsupported
                    ],
                    "other_errors": [{"fixture": n, "error": m} for n, m in other_errors],
                    "expected_error_agreements": [
                        {"fixture": n, "error": m, "origin": o}
                        for n, m, o in expected_error_agreements
                    ],
                    "error_agreement_pandas": error_agreement_pandas,
                    "error_agreement_stamped": error_agreement_stamped,
                    "error_agreement_not_attestable": [
                        {"fixture": n, "origin": o} for n, o in error_agreement_not_pandas
                    ],
                    "uncompared_keys": dict(uncompared_keys),
                    "coverage": dict(how_counts),
                },
                indent=2,
            ),
            encoding="utf-8",
        )
        print(f"\nfull machine-readable report: {args.report_json}")
    if args.apply:
        print(f"\nAPPLIED: {len(moved_attributed)} regenerated, {len(retired)} retired.")
    else:
        print("\nDRY RUN — nothing written.")

    if moved_unattributed:
        print("\nREFUSING to treat unattributed moves as regenerable. Each must be "
              "traced to a named divergence or left failing.", file=sys.stderr)
        return 3
    return 1 if other_errors else 0


if __name__ == "__main__":
    sys.exit(main())
