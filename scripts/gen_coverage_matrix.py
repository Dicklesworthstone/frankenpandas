#!/usr/bin/env python3
"""Generate COVERAGE_MATRIX.md comparing pandas API surface to fixture coverage.

Per br-frankenpandas-zk1j: consumes
`artifacts/pandas_api_listing.json` (from
`scripts/gen_pandas_api_listing.py`) and grep of operation strings
in `crates/fp-conformance/fixtures/packets/*.json`, then writes a
Markdown matrix classifying each pandas API member as:

    Full    → a fixture references it by operation string
    Zero    → no fixture references it

(Partial is reserved for future per-signature coverage.)

Usage:
    python3 scripts/gen_pandas_api_listing.py   # populate listing
    python3 scripts/gen_coverage_matrix.py      # emit COVERAGE_MATRIX.md
"""

from __future__ import annotations

import argparse
import json
import re
from collections import defaultdict
from pathlib import Path


FIXTURE_GLOB = "crates/fp-conformance/fixtures/packets/*.json"
TEST_GLOB = "crates/fp-conformance/src/tests/*.rs"
OPERATION_RE = re.compile(r'"operation"\s*:\s*"([^"]+)"')
DF_BINARY_METHOD_RE = re.compile(r'"dataframe_binary_method"\s*:\s*"([^"]+)"')
INCLUDE_PACKET_RE = re.compile(r'fixtures/packets/([A-Za-z0-9_]+)\.json')
TEST_FN_RE = re.compile(r"^\s*fn\s+([A-Za-z0-9_]+)\s*\(")
COVERS_RE = re.compile(r"//\s*@covers:\s*([^\n]+)")


def canonical_op_for_cover_token(token: str) -> str:
    if "." in token:
        if token.startswith(
            ("Series.str.", "Series.dt.", "Series.cat.", "Series.plot.", "DataFrame.plot.")
        ):
            parts = token.split(".", 2)
            cls_alias = f"{parts[0]}.{parts[1]}"
            mname = parts[2]
        else:
            cls_alias, mname = token.split(".", 1)
        cls_snake = _camel_to_snake(cls_alias.replace(".", "_"))
        return f"{cls_snake}_{mname}"
    return token


def load_fixture_packets(repo_root: Path) -> tuple[set[str], dict[str, list[str]]]:
    ops: set[str] = set()
    packets_by_op: dict[str, set[str]] = defaultdict(set)
    for fixture in sorted(repo_root.glob(FIXTURE_GLOB)):
        try:
            text = fixture.read_text(encoding="utf-8")
        except OSError:
            continue
        stem = fixture.stem
        found_ops = set(OPERATION_RE.findall(text))
        for m in DF_BINARY_METHOD_RE.findall(text):
            found_ops.add(f"dataframe_{m}")
            found_ops.add(f"data_frame_{m}")
        ops.update(found_ops)
        for op in found_ops:
            packets_by_op[op].add(stem)
    return ops, {k: sorted(v) for k, v in packets_by_op.items()}


def load_conformance_tests(
    repo_root: Path,
) -> tuple[dict[str, list[str]], dict[str, list[str]], list[tuple[str, str, bool]]]:
    """Scan crates/fp-conformance/src/tests/*.rs for #[test] functions.

    Returns:
      - live_tests_by_op[op] -> list of unignored live-oracle test IDs
      - skipped_tests_by_op[op] -> list of #[ignore]d or fixture-only skipped test IDs
      - all_test_fns -> list of (test_id, fn_name, is_skipped)
    """
    packet_ops_by_stem: dict[str, set[str]] = defaultdict(set)
    for fixture in sorted(repo_root.glob(FIXTURE_GLOB)):
        try:
            text = fixture.read_text(encoding="utf-8")
        except OSError:
            continue
        found = set(OPERATION_RE.findall(text))
        for m in DF_BINARY_METHOD_RE.findall(text):
            found.add(f"dataframe_{m}")
            found.add(f"data_frame_{m}")
        packet_ops_by_stem[fixture.stem] = found

    live_by_op: dict[str, set[str]] = defaultdict(set)
    skipped_by_op: dict[str, set[str]] = defaultdict(set)
    all_test_fns: list[tuple[str, str, bool]] = []

    for test_file in sorted(repo_root.glob(TEST_GLOB)):
        rel = f"crates/fp-conformance/src/tests/{test_file.name}"
        try:
            lines = test_file.read_text(encoding="utf-8").splitlines()
        except OSError:
            continue
        file_has_live_helper = any(
            marker in "\n".join(lines)
            for marker in (
                "capture_live_oracle_expected",
                "run_pandas_oracle_eval",
                "OracleMode::LiveLegacyPandas",
                "live_oracle_expected_or_skip",
            )
        )
        i = 0
        while i < len(lines):
            if "#[test]" in lines[i]:
                window_start = max(0, i - 3)
                is_ignored = any("#[ignore" in lines[k] for k in range(window_start, min(len(lines), i + 4)))
                j = i + 1
                fn_name = None
                while j < min(len(lines), i + 8):
                    if "#[ignore" in lines[j]:
                        is_ignored = True
                    m = TEST_FN_RE.match(lines[j])
                    if m:
                        fn_name = m.group(1)
                        break
                    j += 1
                if fn_name:
                    body_lines = []
                    depth = 0
                    started = False
                    k = j
                    while k < len(lines):
                        line = lines[k]
                        body_lines.append(line)
                        depth += line.count("{") - line.count("}")
                        if "{" in line:
                            started = True
                        if started and depth <= 0:
                            break
                        k += 1
                    body = "\n".join(body_lines)
                    test_id = f"{rel}::{fn_name}"
                    uses_fixture_only = (
                        "OracleMode::FixtureExpected" in body
                        and "capture_live_oracle_expected" not in body
                        and "OracleMode::LiveLegacyPandas" not in body
                    )
                    is_live_capable = file_has_live_helper and not uses_fixture_only
                    is_skipped = is_ignored or not is_live_capable
                    all_test_fns.append((test_id, fn_name, is_skipped))

                    referenced_ops = set(OPERATION_RE.findall(body))
                    for m in DF_BINARY_METHOD_RE.findall(body):
                        referenced_ops.add(f"dataframe_{m}")
                        referenced_ops.add(f"data_frame_{m}")
                    for stem in INCLUDE_PACKET_RE.findall(body):
                        referenced_ops.update(packet_ops_by_stem.get(stem, set()))
                    for covers_line in COVERS_RE.findall(body):
                        for raw_token in covers_line.split(","):
                            token = raw_token.strip()
                            if token:
                                referenced_ops.add(canonical_op_for_cover_token(token))

                    target_map = skipped_by_op if is_skipped else live_by_op
                    for op in referenced_ops:
                        target_map[op].add(test_id)
                    i = k + 1
                    continue
            i += 1

    return (
        {k: sorted(v) for k, v in live_by_op.items()},
        {k: sorted(v) for k, v in skipped_by_op.items()},
        all_test_fns,
    )


def candidate_op_strings(
    class_alias: str, member_name: str, fixture_ops: set[str] | None = None
) -> set[str]:
    cls_snake = _camel_to_snake(class_alias.replace(".", "_"))
    no_underscore = cls_snake.replace("_", "")
    forms = {
        f"{cls_snake}_{member_name}",
        f"{no_underscore}_{member_name}",
    }
    if cls_snake == "data_frame_group_by":
        forms.update(
            {
                f"dataframe_groupby_{member_name}",
                f"data_frame_groupby_{member_name}",
            }
        )
        if member_name in ("agg", "aggregate"):
            forms.add("dataframe_groupby_agg_multi")
        if member_name == "rolling" and fixture_ops:
            forms.update({o for o in fixture_ops if o.startswith("dataframe_groupby_rolling_")})
    elif cls_snake == "series_group_by":
        forms.update(
            {
                f"groupby_{member_name}",
                f"series_groupby_{member_name}",
                f"series_group_by_{member_name}",
            }
        )
    elif cls_snake == "series":
        forms.add(f"series_{member_name}")
        if member_name == "rolling" and fixture_ops:
            forms.update({o for o in fixture_ops if o.startswith("series_rolling_")})
        elif member_name == "expanding" and fixture_ops:
            forms.update({o for o in fixture_ops if o.startswith("series_expanding_")})
        elif member_name == "resample" and fixture_ops:
            forms.update({o for o in fixture_ops if o.startswith("series_resample_")})
        elif member_name == "ewm" and fixture_ops:
            forms.update({o for o in fixture_ops if o.startswith("series_ewm_")})
    elif cls_snake == "data_frame":
        forms.add(f"dataframe_{member_name}")
        if member_name == "rename":
            forms.add("dataframe_rename_columns")
        elif member_name == "drop":
            forms.add("dataframe_drop_columns")
        elif member_name == "to_json":
            forms.add("dataframe_to_json_records")
        elif member_name == "apply" and fixture_ops:
            forms.update({o for o in fixture_ops if o.startswith("dataframe_apply_")})
        elif member_name == "rolling" and fixture_ops:
            forms.update({o for o in fixture_ops if o.startswith("dataframe_rolling_")})
        elif member_name == "resample" and fixture_ops:
            forms.update({o for o in fixture_ops if o.startswith("dataframe_resample_")})
    elif cls_snake == "rolling":
        forms.add(f"series_rolling_{member_name}")
        forms.add(f"dataframe_rolling_{member_name}")
    elif cls_snake == "expanding":
        forms.add(f"series_expanding_{member_name}")
        forms.add(f"dataframe_expanding_{member_name}")
    elif cls_snake == "resampler":
        forms.add(f"series_resample_{member_name}")
        forms.add(f"dataframe_resample_{member_name}")
    elif cls_snake == "exponential_moving_window":
        forms.add(f"series_ewm_{member_name}")
        forms.add(f"dataframe_ewm_{member_name}")
    elif class_alias == "Series.str":
        forms.update({f"series_str_{member_name}", f"str_{member_name}"})
    elif class_alias == "Series.dt":
        forms.update({f"series_dt_{member_name}", f"dt_{member_name}"})
    elif class_alias == "Series.cat":
        forms.update({f"series_cat_{member_name}", f"cat_{member_name}", f"categorical_{member_name}"})
    elif class_alias == "pandas_top_level":
        forms.update({member_name, f"pandas_{member_name}", f"pd_{member_name}", f"top_level_{member_name}"})
    return forms


def _camel_to_snake(name: str) -> str:
    out = []
    for i, ch in enumerate(name):
        if ch.isupper() and i > 0 and not name[i - 1].isupper() and name[i - 1] != "_":
            out.append("_")
        out.append(ch.lower())
    return "".join(out)


def classify(
    listing: dict,
    fixture_ops: set[str],
    packets_by_op: dict[str, list[str]] | None = None,
    live_by_op: dict[str, list[str]] | None = None,
    skipped_by_op: dict[str, list[str]] | None = None,
    all_test_fns: list[tuple[str, str, bool]] | None = None,
) -> tuple[
    dict[str, dict[str, list[str]]],
    dict[str, dict[str, int]],
    dict[str, dict[str, dict[str, object]]],
    dict[str, dict[str, int]],
]:
    """Returns (breakdown, counts, ledger_members, family_counts)."""
    packets_by_op = packets_by_op or {}
    live_by_op = live_by_op or {}
    skipped_by_op = skipped_by_op or {}
    all_test_fns = all_test_fns or []

    breakdown: dict[str, dict[str, list[str]]] = defaultdict(
        lambda: {"full": [], "packet_only": [], "skipped_only": [], "zero": []}
    )
    counts: dict[str, dict[str, int]] = {}
    ledger_members: dict[str, dict[str, dict[str, object]]] = {}
    family_counts: dict[str, dict[str, int]] = defaultdict(
        lambda: {
            "covered_live": 0,
            "packet_only": 0,
            "skipped_only": 0,
            "uncovered": 0,
            "total": 0,
        }
    )

    for class_alias, entry in listing.get("classes", {}).items():
        if not isinstance(entry, dict) or "members" not in entry:
            continue
        family = str(entry.get("family", "other"))
        ledger_members[class_alias] = {}
        for member in entry["members"]:
            mname = member["name"]
            cands = candidate_op_strings(class_alias, mname, fixture_ops)
            matched_ops = cands & fixture_ops
            packets: set[str] = set()
            live_tests: set[str] = set()
            skipped_tests: set[str] = set()
            for op in matched_ops:
                packets.update(packets_by_op.get(op, []))
                live_tests.update(live_by_op.get(op, []))
                skipped_tests.update(skipped_by_op.get(op, []))
            for op in cands:
                live_tests.update(live_by_op.get(op, []))
                skipped_tests.update(skipped_by_op.get(op, []))

            # Also match conformance/live_oracle test function names directly
            for test_id, fn_name, is_skipped in all_test_fns:
                for cand in cands:
                    if (
                        fn_name.startswith(f"live_oracle_{cand}")
                        or fn_name.startswith(f"conformance_{cand}")
                    ):
                        if is_skipped:
                            skipped_tests.add(test_id)
                        else:
                            live_tests.add(test_id)

            # Anti-gaming rule (j0lg5.1): a member counts as covered_live ONLY when at least one
            # unignored live-oracle test exercises it. Skipped/ignored-only tests never count as covered.
            if live_tests:
                last_result = "covered_live"
                breakdown[class_alias]["full"].append(mname)
            elif packets or matched_ops:
                last_result = "packet_only"
                breakdown[class_alias]["packet_only"].append(mname)
                breakdown[class_alias]["full"].append(mname)
            elif skipped_tests:
                last_result = "skipped_only"
                breakdown[class_alias]["skipped_only"].append(mname)
                breakdown[class_alias]["zero"].append(mname)
            else:
                last_result = "uncovered"
                breakdown[class_alias]["zero"].append(mname)

            ledger_members[class_alias][mname] = {
                "packets": sorted(packets),
                "live_tests": sorted(live_tests),
                "skipped_tests": sorted(skipped_tests),
                "last_result": last_result,
            }
            family_counts[family][last_result] += 1
            family_counts[family]["total"] += 1

        full_n = len(breakdown[class_alias]["full"])
        zero_n = len(breakdown[class_alias]["zero"])
        live_n = sum(
            1
            for m in ledger_members[class_alias].values()
            if m["last_result"] == "covered_live"
        )
        pkt_only_n = sum(
            1
            for m in ledger_members[class_alias].values()
            if m["last_result"] == "packet_only"
        )
        skip_only_n = sum(
            1
            for m in ledger_members[class_alias].values()
            if m["last_result"] == "skipped_only"
        )
        counts[class_alias] = {
            "family": family,
            "covered_live": live_n,
            "packet_only": pkt_only_n,
            "skipped_only": skip_only_n,
            "full": full_n,
            "zero": zero_n,
            "total": full_n + zero_n,
        }
    return breakdown, counts, ledger_members, dict(family_counts)


def verify_negative_skipped_test_not_counted_as_covered() -> None:
    """NEGATIVE check (j0lg5.1): a member exercised ONLY by a skipped/#[ignore]d test
    must be classified as skipped_only (zero / not covered), never covered_live or full.
    """
    synthetic_listing = {
        "pandas_version": "2.2.3",
        "classes": {
            "Series": {
                "family": "core",
                "members": [
                    {"name": "only_skipped_member", "kind": "method"},
                    {"name": "live_covered_member", "kind": "method"},
                ],
            }
        },
    }
    breakdown, counts, ledger, fam = classify(
        synthetic_listing,
        fixture_ops=set(),
        packets_by_op={},
        live_by_op={"series_live_covered_member": ["crates/fp-conformance/src/tests/t.rs::live_ok"]},
        skipped_by_op={
            "series_only_skipped_member": ["crates/fp-conformance/src/tests/t.rs::ignored_test"]
        },
        all_test_fns=[
            (
                "crates/fp-conformance/src/tests/t.rs::live_oracle_series_only_skipped_member",
                "live_oracle_series_only_skipped_member",
                True,
            )
        ],
    )
    skipped_entry = ledger["Series"]["only_skipped_member"]
    assert skipped_entry["last_result"] == "skipped_only", skipped_entry
    assert "only_skipped_member" not in breakdown["Series"]["full"]
    assert "only_skipped_member" in breakdown["Series"]["zero"]
    assert counts["Series"]["covered_live"] == 1
    assert counts["Series"]["skipped_only"] == 1
    assert fam["core"]["covered_live"] == 1
    assert fam["core"]["skipped_only"] == 1
    assert canonical_op_for_cover_token("DataFrameGroupBy.mean") == "data_frame_group_by_mean"
    assert canonical_op_for_cover_token("Series.str.contains") == "series_str_contains"


def render(breakdown, counts, family_counts, pandas_version: str) -> str:
    lines: list[str] = []
    lines.append("# Coverage Matrix")
    lines.append("")
    lines.append(
        f"Auto-generated by `scripts/gen_coverage_matrix.py` against "
        f"pandas **{pandas_version}** (from `artifacts/pandas_api_listing.json` and "
        "`artifacts/api_coverage_ledger.json`). Re-run both scripts after upgrading pandas."
    )
    lines.append("")
    lines.append("## Per-Family Summary")
    lines.append("")
    lines.append("| Family | Covered (Live Oracle) | Packet Only | Skipped Only (Not Covered) | Uncovered | Total | Live Coverage | Any-Packet/Live |")
    lines.append("|---|---:|---:|---:|---:|---:|---:|---:|")
    fam_live = fam_pkt = fam_skip = fam_uncov = 0
    for fam, fc in sorted(family_counts.items()):
        tot = fc["total"]
        live_pct = (fc["covered_live"] / tot * 100.0) if tot else 0.0
        any_pct = ((fc["covered_live"] + fc["packet_only"]) / tot * 100.0) if tot else 0.0
        lines.append(
            f"| `{fam}` | {fc['covered_live']} | {fc['packet_only']} | {fc['skipped_only']} | "
            f"{fc['uncovered']} | {tot} | {live_pct:.1f}% | {any_pct:.1f}% |"
        )
        fam_live += fc["covered_live"]
        fam_pkt += fc["packet_only"]
        fam_skip += fc["skipped_only"]
        fam_uncov += fc["uncovered"]
    fam_tot = fam_live + fam_pkt + fam_skip + fam_uncov
    fam_live_pct = (fam_live / fam_tot * 100.0) if fam_tot else 0.0
    fam_any_pct = ((fam_live + fam_pkt) / fam_tot * 100.0) if fam_tot else 0.0
    lines.append(
        f"| **ALL FAMILIES** | **{fam_live}** | **{fam_pkt}** | **{fam_skip}** | "
        f"**{fam_uncov}** | **{fam_tot}** | **{fam_live_pct:.1f}%** | **{fam_any_pct:.1f}%** |"
    )
    lines.append("")
    lines.append("## Per-Surface Summary")
    lines.append("")
    lines.append("| Surface | Family | Covered (Live) | Packet Only | Skipped Only | Uncovered | Total | Live % | Any % |")
    lines.append("|---|---|---:|---:|---:|---:|---:|---:|---:|")
    total_full = total_zero = 0
    for cls, c in sorted(counts.items()):
        pct = (c["full"] / c["total"] * 100) if c["total"] else 0.0
        live_pct = (c["covered_live"] / c["total"] * 100) if c["total"] else 0.0
        uncov = c["zero"] - c["skipped_only"]
        lines.append(
            f"| {cls} | `{c['family']}` | {c['covered_live']} | {c['packet_only']} | "
            f"{c['skipped_only']} | {uncov} | {c['total']} | {live_pct:.1f}% | {pct:.1f}% |"
        )
        total_full += c["full"]
        total_zero += c["zero"]
    total = total_full + total_zero
    total_pct = (total_full / total * 100) if total else 0.0
    lines.append(
        f"| **ALL** | **all** | **{fam_live}** | **{fam_pkt}** | **{fam_skip}** | "
        f"**{fam_uncov}** | **{total}** | **{fam_live_pct:.1f}%** | **{total_pct:.1f}%** |"
    )
    lines.append("")
    lines.append("## Per-class breakdown")
    for cls, b in sorted(breakdown.items()):
        lines.append("")
        lines.append(f"### {cls}")
        full = sorted(b["full"])
        zero = sorted(b["zero"])
        lines.append("")
        lines.append(f"**Full ({len(full)}):** {', '.join(full) if full else '_none_'}")
        lines.append("")
        lines.append(f"**Zero ({len(zero)}):** {', '.join(zero) if zero else '_none_'}")
    lines.append("")
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description="Generate COVERAGE_MATRIX.md and api_coverage_ledger.json")
    parser.add_argument(
        "--listing",
        default="artifacts/pandas_api_listing.json",
        help="Input listing JSON (default: artifacts/pandas_api_listing.json)",
    )
    parser.add_argument(
        "--out",
        default="docs/planning/COVERAGE_MATRIX.md",
        help="Output Markdown path (default: docs/planning/COVERAGE_MATRIX.md)",
    )
    parser.add_argument(
        "--ledger-out",
        default="artifacts/api_coverage_ledger.json",
        help="Output per-member JSON ledger path (default: artifacts/api_coverage_ledger.json)",
    )
    args = parser.parse_args()

    verify_negative_skipped_test_not_counted_as_covered()

    repo_root = Path(__file__).resolve().parents[1]
    listing_path = Path(args.listing)
    if not listing_path.is_absolute():
        listing_path = repo_root / listing_path
    try:
        with listing_path.open(encoding="utf-8") as listing_file:
            listing = json.load(listing_file)
    except (OSError, json.JSONDecodeError) as exc:
        raise SystemExit(f"failed to load pandas API listing {listing_path}: {exc}") from exc

    fixture_ops, packets_by_op = load_fixture_packets(repo_root)
    live_by_op, skipped_by_op, all_test_fns = load_conformance_tests(repo_root)
    breakdown, counts, ledger_members, family_counts = classify(
        listing, fixture_ops, packets_by_op, live_by_op, skipped_by_op, all_test_fns
    )

    pandas_version = str(listing.get("pandas_version", "?"))
    rendered = render(breakdown, counts, family_counts, pandas_version)
    out_path = Path(args.out)
    if not out_path.is_absolute():
        out_path = repo_root / out_path
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(rendered, encoding="utf-8")

    ledger_path = Path(args.ledger_out)
    if not ledger_path.is_absolute():
        ledger_path = repo_root / ledger_path
    ledger_path.parent.mkdir(parents=True, exist_ok=True)
    total_live = sum(c["covered_live"] for c in counts.values())
    total_pkt = sum(c["packet_only"] for c in counts.values())
    total_skip = sum(c["skipped_only"] for c in counts.values())
    total_full = sum(c["full"] for c in counts.values())
    total_zero = sum(c["zero"] for c in counts.values())
    total = total_full + total_zero
    ledger_payload = {
        "generated_by": "scripts/gen_coverage_matrix.py",
        "pandas_version": pandas_version,
        "summary": {
            "surface_count": len(counts),
            "total_members": total,
            "covered_live": total_live,
            "packet_only": total_pkt,
            "skipped_only": total_skip,
            "uncovered": total_zero - total_skip,
            "live_coverage_percent": round((total_live / total * 100.0) if total else 0.0, 2),
            "packet_or_live_coverage_percent": round((total_full / total * 100.0) if total else 0.0, 2),
            "by_family": family_counts,
            "by_surface": counts,
        },
        "members": ledger_members,
    }
    ledger_path.write_text(json.dumps(ledger_payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    pct = (total_full / total * 100) if total else 0.0
    live_pct = (total_live / total * 100) if total else 0.0
    fam_str = ", ".join(
        f"{fam}:live={fc['covered_live']}/pkt={fc['packet_only']}/tot={fc['total']}"
        for fam, fc in sorted(family_counts.items())
    )
    print(
        f"coverage_matrix: surfaces={len(counts)} "
        f"covered_live={total_live} ({live_pct:.1f}%) packet_only={total_pkt} "
        f"full={total_full} ({pct:.1f}%) zero={total_zero} total={total} "
        f"[{fam_str}] → {out_path}, {ledger_path}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
