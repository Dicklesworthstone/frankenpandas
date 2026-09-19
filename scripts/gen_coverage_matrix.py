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
OPERATION_RE = re.compile(r'"operation"\s*:\s*"([^"]+)"')
DF_BINARY_METHOD_RE = re.compile(r'"dataframe_binary_method"\s*:\s*"([^"]+)"')


def load_fixture_ops(repo_root: Path) -> set[str]:
    ops: set[str] = set()
    for fixture in repo_root.glob(FIXTURE_GLOB):
        try:
            text = fixture.read_text()
        except OSError:
            continue
        ops.update(OPERATION_RE.findall(text))
        for m in DF_BINARY_METHOD_RE.findall(text):
            ops.add(f"dataframe_{m}")
            ops.add(f"data_frame_{m}")
    return ops


# Map a pandas {class, member} pair to the operation strings our fixtures
# and oracle dispatch might carry. Multiple alias forms are common
# (e.g. "series_add", "data_frame_add"). Heuristic normalization handles
# the two dominant spellings.
def candidate_op_strings(
    class_alias: str, member_name: str, fixture_ops: set[str] | None = None
) -> set[str]:
    cls_snake = _camel_to_snake(class_alias)
    no_underscore = cls_snake.replace("_", "")
    forms = {
        f"{cls_snake}_{member_name}",
        f"{no_underscore}_{member_name}",
    }
    # GroupBy packets use the oracle dispatch spellings rather than the
    # literal pandas class names.
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
    return forms


def _camel_to_snake(name: str) -> str:
    # CamelCase → camel_case
    out = []
    for i, ch in enumerate(name):
        if ch.isupper() and i > 0 and not name[i - 1].isupper():
            out.append("_")
        out.append(ch.lower())
    return "".join(out)


def classify(
    listing: dict, fixture_ops: set[str]
) -> tuple[dict[str, dict[str, list[str]]], dict[str, dict[str, int]]]:
    """Returns (breakdown, counts):
    breakdown[class] = {"full": [...], "zero": [...]}
    counts[class] = {"full": N, "zero": N, "total": N}
    """
    breakdown: dict[str, dict[str, list[str]]] = defaultdict(
        lambda: {"full": [], "zero": []}
    )
    counts: dict[str, dict[str, int]] = {}
    for class_alias, entry in listing.get("classes", {}).items():
        if not isinstance(entry, dict) or "members" not in entry:
            continue
        for member in entry["members"]:
            cands = candidate_op_strings(class_alias, member["name"], fixture_ops)
            hit = bool(cands & fixture_ops)
            bucket = "full" if hit else "zero"
            breakdown[class_alias][bucket].append(member["name"])
        full_n = len(breakdown[class_alias]["full"])
        zero_n = len(breakdown[class_alias]["zero"])
        counts[class_alias] = {
            "full": full_n,
            "zero": zero_n,
            "total": full_n + zero_n,
        }
    return breakdown, counts


def render(breakdown, counts, pandas_version: str) -> str:
    lines: list[str] = []
    lines.append("# Coverage Matrix")
    lines.append("")
    lines.append(
        f"Auto-generated by `scripts/gen_coverage_matrix.py` against "
        f"pandas **{pandas_version}** (from `artifacts/pandas_api_listing.json`). "
        "Re-run both scripts after upgrading pandas."
    )
    lines.append("")
    lines.append("## Summary")
    lines.append("")
    lines.append("| Class | Full | Zero | Total | Coverage |")
    lines.append("|---|---:|---:|---:|---:|")
    total_full = total_zero = 0
    for cls, c in sorted(counts.items()):
        pct = (c["full"] / c["total"] * 100) if c["total"] else 0.0
        lines.append(
            f"| {cls} | {c['full']} | {c['zero']} | {c['total']} | {pct:.1f}% |"
        )
        total_full += c["full"]
        total_zero += c["zero"]
    total = total_full + total_zero
    total_pct = (total_full / total * 100) if total else 0.0
    lines.append(
        f"| **ALL** | **{total_full}** | **{total_zero}** | **{total}** | "
        f"**{total_pct:.1f}%** |"
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
    parser = argparse.ArgumentParser(description="Generate COVERAGE_MATRIX.md")
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
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parents[1]
    listing_path = Path(args.listing)
    if not listing_path.is_absolute():
        listing_path = repo_root / listing_path
    try:
        with listing_path.open(encoding="utf-8") as listing_file:
            listing = json.load(listing_file)
    except (OSError, json.JSONDecodeError) as exc:
        raise SystemExit(f"failed to load pandas API listing {listing_path}: {exc}") from exc

    fixture_ops = load_fixture_ops(repo_root)
    breakdown, counts = classify(listing, fixture_ops)

    rendered = render(breakdown, counts, listing.get("pandas_version", "?"))
    out_path = Path(args.out)
    if not out_path.is_absolute():
        out_path = repo_root / out_path
    out_path.write_text(rendered)

    total_full = sum(c["full"] for c in counts.values())
    total_zero = sum(c["zero"] for c in counts.values())
    total = total_full + total_zero
    pct = (total_full / total * 100) if total else 0.0
    print(
        f"coverage_matrix: classes={len(counts)} "
        f"full={total_full} zero={total_zero} total={total} pct={pct:.1f}% → {out_path}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
