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


def classify_move(pinned: Any, live: Any) -> tuple[str, str]:
    """Bucket a moved value into a countable class.

    Returns (class, example). Classification is STRUCTURAL — it walks the trees
    rather than parsing a human-readable mismatch string — so the counts do not
    depend on message formatting.
    """
    # Compare the kind-normalized trees so serde aliases never register.
    leaves = differing_leaves(normalize_structural(pinned), normalize_structural(live))
    if not leaves:
        return "NORMALIZES_EQUAL", ""

    kinds: set[str] = set()
    for path, p, l in leaves:
        # A differing {"kind": ...} pair is the signal we care about; walk up by
        # inspecting the sibling structures the leaf came from.
        if path.endswith(".kind") and isinstance(p, str) and isinstance(l, str):
            kinds.add(f"{p}->{l}")
        elif path.endswith(".value"):
            kinds.add("value")
    path, p, l = leaves[0]
    example = f"{path}: {p!r} vs {l!r}"

    promo = {k for k in kinds if "->" in k and k != "null->null"}
    if promo:
        # e.g. int64->float64: dtype promotion, DISC-011 territory.
        return f"KIND {sorted(promo)[0]}", example
    if any(path.endswith(".value") for path, _, _ in leaves) and _null_marker_move(leaves):
        direction = _null_marker_move(leaves)
        return f"NULL_MARKER {direction}", example
    if any(p == "<absent>" or l == "<absent>" for _, p, l in leaves):
        return "SHAPE absent-key", example
    if any(path.endswith("[len]") for path, _, _ in leaves):
        return "SHAPE length", example
    return "VALUE", example


def _null_marker_move(leaves: list[tuple[str, Any, Any]]) -> str:
    """Direction of a null-discriminator move, or empty string if not one."""
    markers = {"null", "na_n", "nan"}
    for _, p, l in leaves:
        if isinstance(p, str) and isinstance(l, str) and p in markers and l in markers:
            return f"{p}->{l}"
    return ""


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


def run_oracle(fixture: dict[str, Any], legacy_root: str, timeout: int) -> dict[str, Any]:
    proc = subprocess.run(
        [sys.executable, str(ORACLE), "--legacy-root", legacy_root,
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
    present = [k for k in fixture if k.startswith("expected")]
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
    parser.add_argument("--glob", default="*.json")
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--jobs", type=int, default=1,
                        help="concurrent oracle invocations; report stays deterministic")
    parser.add_argument("--list-limit", type=int, default=20,
                        help="how many individual moves to print (class COUNTS are always complete)")
    parser.add_argument("--report-json", type=Path,
                        help="write the COMPLETE machine-readable report: every move with its "
                             "class, every error, no truncation. Required input for an "
                             "attribution pass driven by counts rather than by a visible slice.")
    args = parser.parse_args()

    if args.apply and args.attributions is None:
        print("--apply requires --attributions: bulk regeneration is forbidden "
              "(p6srr / maintainer ruling)", file=sys.stderr)
        return 2
    allow = load_attributions(args.attributions)

    fixtures = sorted(FIXTURE_ROOT.glob(args.glob))
    if args.limit:
        fixtures = fixtures[: args.limit]

    def examine(path: Path):
        fixture = json.loads(path.read_text(encoding="utf-8"))
        try:
            response = run_oracle(fixture, args.legacy_root, args.timeout)
        except Exception as exc:  # noqa: BLE001
            return path, fixture, None, str(exc)[:160]
        if response.get("error"):
            return path, fixture, None, f"oracle error: {response['error']}"
        return path, fixture, response, None

    if args.jobs > 1:
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
            outcomes = list(pool.map(examine, fixtures))
    else:
        outcomes = [examine(p) for p in fixtures]

    agreed = prov_only = 0
    moved_attributed: list[tuple[str, str]] = []
    moved_unattributed: list[tuple[str, list[str], str, str, str]] = []
    move_classes: collections.Counter[str] = collections.Counter()
    retired: list[tuple[str, str]] = []
    unsupported: list[tuple[str, str]] = []
    other_errors: list[tuple[str, str]] = []
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
                klass, example = "UNCLASSIFIED", ""
                for key in verdict["moved"]:
                    if key in fixture and key in response:
                        klass, example = classify_move(fixture[key], response[key])
                        break
                moved_unattributed.append(
                    (name, verdict["moved"], verdict["detail"].get("semantic", "")[:110],
                     klass, example))
                move_classes[klass] += 1
        elif verdict["provenance_stale"]:
            prov_only += 1
        else:
            agreed += 1

    print(f"fixtures examined            : {len(fixtures)}")
    print(f"  agree, fully current       : {agreed}")
    print(f"  agree, provenance-only     : {prov_only}")
    print(f"  MOVED, attributed          : {len(moved_attributed)}")
    print(f"  MOVED, UNATTRIBUTED        : {len(moved_unattributed)}   <-- stay failing fixtures")
    print(f"  oracle: unsupported op     : {len(unsupported)}   <-- retire candidates")
    print(f"  oracle: other errors       : {len(other_errors)}")
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
        print(f"\nMOVE CLASSES (all {len(moved_unattributed)}, largest first):")
        for klass, count in move_classes.most_common():
            share = 100.0 * count / len(moved_unattributed)
            print(f"  {count:5d}  {share:5.1f}%  {klass}")
        print(f"\nUNATTRIBUTED MOVES (first {args.list_limit} of "
              f"{len(moved_unattributed)}; use --report-json for all):")
        for name, keys, why, klass, example in moved_unattributed[: args.list_limit]:
            print(f"  [{klass}] {name}: {', '.join(keys)}  {why or example}")
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
                    "move_classes": dict(move_classes),
                    "moved_unattributed": [
                        {"fixture": n, "keys": k, "why": w, "class": c, "example": e}
                        for n, k, w, c, e in moved_unattributed
                    ],
                    "moved_attributed": [
                        {"fixture": n, "divergence": d} for n, d in moved_attributed
                    ],
                    "unsupported_operation": [
                        {"fixture": n, "error": m} for n, m in unsupported
                    ],
                    "other_errors": [{"fixture": n, "error": m} for n, m in other_errors],
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
