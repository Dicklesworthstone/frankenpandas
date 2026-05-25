#!/usr/bin/env python3
"""Feature Universe CI gate for FrankenPandas surface coverage.

Compares new feature universe measurements against the committed baseline
and enforces regression thresholds:

- functional_pct must not drop below 98%
- missing count must not grow beyond 30 without waiver

Verdicts:
- ALLOW: All thresholds pass
- BLOCK: Regression detected, CI fails
- WAIVER: Regression detected but waiver file exists

Usage:
    python scripts/feature_universe_gate.py --baseline artifacts/feature_universe.json --new new_universe.json
    python scripts/feature_universe_gate.py --update-baseline new_universe.json
"""
from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

PROJECT_ROOT = Path(__file__).parent.parent
BASELINE_PATH = PROJECT_ROOT / "artifacts" / "feature_universe.json"
WAIVER_PATH = PROJECT_ROOT / "artifacts" / "feature_universe_waivers.json"


def load_json(path: Path) -> dict[str, Any]:
    with open(path) as f:
        return json.load(f)


def save_json(path: Path, data: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "w") as f:
        json.dump(data, f, indent=2)
    print(f"Saved: {path}")


def compare_universe(baseline: dict, new: dict) -> tuple[str, dict]:
    """Compare new universe against baseline, return verdict and report."""
    b_summary = baseline.get("summary", {})
    n_summary = new.get("summary", {})

    b_functional = b_summary.get("functional_pct", 0)
    n_functional = n_summary.get("functional_pct", 0)
    b_missing = b_summary.get("missing", 0)
    n_missing = n_summary.get("missing", 0)

    thresholds = baseline.get("thresholds", {})
    min_functional = thresholds.get("functional_pct_min", 98.0)
    max_missing = thresholds.get("missing_max", 30)

    violations = []
    if n_functional < min_functional:
        violations.append(f"functional_pct dropped to {n_functional}% (min: {min_functional}%)")
    if n_missing > max_missing:
        violations.append(f"missing count grew to {n_missing} (max: {max_missing})")
    if n_functional < b_functional:
        violations.append(f"functional_pct regressed from {b_functional}% to {n_functional}%")
    if n_missing > b_missing:
        violations.append(f"missing count grew from {b_missing} to {n_missing}")

    category_changes = []
    b_cats = baseline.get("categories", {})
    n_cats = new.get("categories", {})
    for cat in set(b_cats.keys()) | set(n_cats.keys()):
        b_cat = b_cats.get(cat, {})
        n_cat = n_cats.get(cat, {})
        b_present = b_cat.get("present", 0)
        n_present = n_cat.get("present", 0)
        if n_present < b_present:
            category_changes.append({
                "category": cat,
                "change": "regression",
                "before": b_present,
                "after": n_present,
            })
        elif n_present > b_present:
            category_changes.append({
                "category": cat,
                "change": "improvement",
                "before": b_present,
                "after": n_present,
            })

    report = {
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "baseline_functional_pct": b_functional,
        "new_functional_pct": n_functional,
        "baseline_missing": b_missing,
        "new_missing": n_missing,
        "violations": violations,
        "category_changes": category_changes,
        "thresholds": {
            "functional_pct_min": min_functional,
            "missing_max": max_missing,
        },
    }

    if violations:
        if WAIVER_PATH.exists():
            waivers = load_json(WAIVER_PATH)
            if waivers.get("active"):
                report["waiver"] = waivers
                return "WAIVER", report
        return "BLOCK", report
    return "ALLOW", report


def update_baseline(new_path: Path) -> None:
    """Copy new universe as the new baseline."""
    new = load_json(new_path)
    new["generated_at"] = datetime.now(timezone.utc).isoformat()
    save_json(BASELINE_PATH, new)


def main():
    parser = argparse.ArgumentParser(description="Feature Universe CI gate")
    parser.add_argument("--baseline", type=Path, help="Path to baseline JSON")
    parser.add_argument("--new", type=Path, help="Path to new universe JSON")
    parser.add_argument("--update-baseline", type=Path, help="Update baseline with this file")
    parser.add_argument("--json", action="store_true", help="Output JSON instead of text")
    args = parser.parse_args()

    if args.update_baseline:
        update_baseline(args.update_baseline)
        print("ALLOW: Baseline updated")
        return 0

    baseline_path = args.baseline or BASELINE_PATH
    if not baseline_path.exists():
        print(f"ALLOW: No baseline at {baseline_path}, skipping gate")
        return 0

    if not args.new:
        parser.error("--new is required for comparison")

    if not args.new.exists():
        print(f"Error: {args.new} not found", file=sys.stderr)
        return 1

    baseline = load_json(baseline_path)
    new = load_json(args.new)

    verdict, report = compare_universe(baseline, new)

    if args.json:
        print(json.dumps(report, indent=2))
    else:
        print(f"\n{'='*60}")
        print(f"FEATURE UNIVERSE GATE: {verdict}")
        print(f"{'='*60}\n")
        print(f"Functional coverage: {report['baseline_functional_pct']}% -> {report['new_functional_pct']}%")
        print(f"Missing APIs: {report['baseline_missing']} -> {report['new_missing']}")

        if report["violations"]:
            print(f"\nViolations:")
            for v in report["violations"]:
                print(f"  - {v}")

        if report["category_changes"]:
            print(f"\nCategory changes:")
            for c in report["category_changes"]:
                print(f"  - {c['category']}: {c['before']} -> {c['after']} ({c['change']})")

        print(f"\nVerdict: {verdict}")

    if verdict == "BLOCK":
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
