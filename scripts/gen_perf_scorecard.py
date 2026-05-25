#!/usr/bin/env python3
"""Generate per-category speedup scorecard from benchmark results.

Per bead 7.5: produces an honest per-category vs-pandas scorecard.
Where FrankenPandas exceeds pandas: states the measured ratio.
Where it does NOT: states so plainly.

Usage:
    python scripts/gen_perf_scorecard.py --input artifacts/bench/latest.json
    python scripts/gen_perf_scorecard.py --input artifacts/bench/latest.json --format md
"""
from __future__ import annotations

import argparse
import json
import math
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

PROJECT_ROOT = Path(__file__).parent.parent
ARTIFACTS_DIR = PROJECT_ROOT / "artifacts" / "perf"

CATEGORIES = {
    "io": {"weight": 0.25, "name": "IO"},
    "dataframe_ops": {"weight": 0.20, "name": "DataFrameOps"},
    "groupby": {"weight": 0.20, "name": "GroupBy"},
    "joins": {"weight": 0.15, "name": "Joins"},
    "rolling": {"weight": 0.10, "name": "Rolling/Expanding"},
    "indexing": {"weight": 0.10, "name": "Indexing"},
}


def load_json(path: Path) -> dict[str, Any]:
    with open(path) as f:
        return json.load(f)


def save_json(path: Path, data: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "w") as f:
        json.dump(data, f, indent=2)


def compute_geomean(values: list[float]) -> float:
    if not values or any(v <= 0 for v in values):
        return 1.0
    return math.exp(sum(math.log(v) for v in values) / len(values))


def classify_ratio(ratio: float) -> str:
    """Classify speedup ratio into verdict."""
    if ratio > 1.05:
        return "FASTER"
    elif ratio >= 0.95:
        return "PARITY"
    else:
        return "SLOWER"


def generate_scorecard(results: list[dict]) -> dict[str, Any]:
    """Generate per-category scorecard from benchmark results."""
    by_category: dict[str, list[float]] = {cat: [] for cat in CATEGORIES}

    for r in results:
        cat = r.get("category")
        ratio = r.get("ratio")
        if cat in by_category and ratio is not None and ratio > 0:
            by_category[cat].append(ratio)

    categories = []
    for cat_id, cat_info in CATEGORIES.items():
        ratios = by_category[cat_id]
        geomean = compute_geomean(ratios)
        verdict = classify_ratio(geomean)

        categories.append({
            "category": cat_info["name"],
            "category_id": cat_id,
            "weight": cat_info["weight"],
            "workload_count": len(ratios),
            "geomean_ratio": round(geomean, 2),
            "verdict": verdict,
            "exceeds_pandas": verdict == "FASTER",
        })

    weighted_score = sum(
        c["geomean_ratio"] * c["weight"]
        for c in categories
    )

    claim_validated = all(c["exceeds_pandas"] for c in categories)
    exceeds_count = sum(1 for c in categories if c["exceeds_pandas"])

    return {
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "categories": categories,
        "weighted_score": round(weighted_score, 2),
        "claim_validated": claim_validated,
        "exceeds_count": exceeds_count,
        "total_categories": len(categories),
        "summary": f"{exceeds_count}/{len(categories)} categories exceed pandas",
    }


def format_markdown_scorecard(scorecard: dict) -> str:
    """Format scorecard as markdown table."""
    lines = [
        "## Performance Scorecard",
        "",
        f"Generated: {scorecard['timestamp'][:10]}",
        "",
        "| Category | Weight | Ratio | Verdict |",
        "|----------|--------|-------|---------|",
    ]

    for c in scorecard["categories"]:
        ratio_str = f"{c['geomean_ratio']:.2f}x"
        lines.append(f"| {c['category']} | {c['weight']} | {ratio_str} | {c['verdict']} |")

    lines.extend([
        f"| **Weighted** | **1.00** | **{scorecard['weighted_score']:.2f}x** | {'**PASS**' if scorecard['claim_validated'] else '**PARTIAL**'} |",
        "",
        f"**Summary**: {scorecard['summary']}",
        "",
    ])

    if not scorecard["claim_validated"]:
        slower_cats = [c for c in scorecard["categories"] if c["verdict"] == "SLOWER"]
        if slower_cats:
            lines.append("### Categories Below Parity")
            lines.append("")
            for c in slower_cats:
                lines.append(f"- **{c['category']}**: {c['geomean_ratio']:.2f}x (needs optimization)")
            lines.append("")

    return "\n".join(lines)


def format_text_scorecard(scorecard: dict) -> str:
    """Format scorecard as plain text."""
    lines = [
        "=" * 60,
        "FRANKENPANDAS vs PANDAS PERFORMANCE SCORECARD",
        "=" * 60,
        f"Generated: {scorecard['timestamp']}",
        "",
    ]

    lines.append(f"{'Category':<20} {'Weight':<8} {'Ratio':<10} {'Verdict':<10}")
    lines.append("-" * 60)

    for c in scorecard["categories"]:
        lines.append(
            f"{c['category']:<20} {c['weight']:<8} {c['geomean_ratio']:.2f}x{'':<6} {c['verdict']:<10}"
        )

    lines.extend([
        "-" * 60,
        f"{'WEIGHTED TOTAL':<20} {'1.00':<8} {scorecard['weighted_score']:.2f}x",
        "",
        f"Claim validated: {'YES' if scorecard['claim_validated'] else 'NO'}",
        f"Summary: {scorecard['summary']}",
    ])

    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description="Generate performance scorecard")
    parser.add_argument("--input", type=Path, required=True, help="Benchmark results JSON")
    parser.add_argument("--output", type=Path, help="Output file")
    parser.add_argument(
        "--format",
        choices=["json", "md", "text"],
        default="text",
        help="Output format"
    )
    args = parser.parse_args()

    if not args.input.exists():
        print(f"Error: {args.input} not found", file=sys.stderr)
        return 1

    data = load_json(args.input)
    results = data.get("results", [])

    if not results:
        print("Error: No results found in input file", file=sys.stderr)
        return 1

    scorecard = generate_scorecard(results)

    if args.format == "json":
        output = json.dumps(scorecard, indent=2)
    elif args.format == "md":
        output = format_markdown_scorecard(scorecard)
    else:
        output = format_text_scorecard(scorecard)

    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(output)
        print(f"Scorecard written to: {args.output}")
    else:
        print(output)

    if not scorecard["claim_validated"]:
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
