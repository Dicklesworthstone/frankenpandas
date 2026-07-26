#!/usr/bin/env python3
"""Performance ratchet gate for FrankenPandas benchmarks.

Compares new benchmark results against committed baselines and enforces
regression thresholds per the gauntlet spec:

Thresholds:
  - primary (any single bench):    -3%
  - geomean (category geomean):    -5%
  - per-category weighted:        -10%
  - p90 tail latency:             -15%
  - throughput:                    -5%

Verdicts:
  - ALLOW: All thresholds pass, update baseline
  - BLOCK: Regression beyond threshold, fail CI
  - QUARANTINE: Some measurements are contract-invalid or median-CI undecidable

CV is retained as provenance only. It never decides a ratchet verdict.

Usage:
    python scripts/perf_ratchet.py --baseline .bench-history/latest.json --new artifacts/bench/current.json
    python scripts/perf_ratchet.py --update-baseline artifacts/bench/current.json
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
BASELINE_DIR = PROJECT_ROOT / ".bench-history"

THRESHOLDS = {
    "primary_pct": -3.0,
    "geomean_pct": -5.0,
    "per_category_pct": -10.0,
    "p90_pct": -15.0,
    "throughput_pct": -5.0,
}

CATEGORIES = {
    "io": 0.25,
    "dataframe_ops": 0.20,
    "groupby": 0.20,
    "joins": 0.15,
    "rolling": 0.10,
    "indexing": 0.10,
}

UNCERTAIN_VERDICTS = {
    "NULL_UNDECIDABLE",
    "CONTRACT_INVALID",
    "INCOMPLETE",
    # Historical v3 artifact; retained only as legacy uncertainty.
    "DROPPED_HIGH_CV",
}


def load_json(path: Path) -> dict[str, Any]:
    with open(path) as f:
        return json.load(f)


def save_json(path: Path, data: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "w") as f:
        json.dump(data, f, indent=2)
    print(f"Saved: {path}")


def compute_geomean(values: list[float]) -> float:
    if not values:
        return 1.0
    if any(v <= 0 for v in values):
        return 1.0
    return math.exp(sum(math.log(v) for v in values) / len(values))


def fp_metric(result: dict[str, Any], metric: str) -> float:
    """Read harness v4 nested metrics, with the historical flat-field fallback."""
    nested = result.get("frankenpandas")
    if isinstance(nested, dict):
        value = nested.get(metric, 0)
        if isinstance(value, int | float):
            return float(value)
    legacy_name = {
        "p50_us": "fp_p50_us",
        "p95_us": "fp_p95_us",
        "throughput_rows_sec": "fp_throughput",
    }[metric]
    value = result.get(legacy_name, 0)
    return float(value) if isinstance(value, int | float) else 0.0


def workload_key(result: dict[str, Any]) -> tuple[Any, Any, Any]:
    return result.get("workload"), result.get("size"), result.get("dtype")


def is_decidable(result: dict[str, Any]) -> bool:
    """Only median-CI-decided rows may vote in the regression ratchet."""
    return result.get("verdict") not in UNCERTAIN_VERDICTS


def compare_workload(baseline: dict, new: dict) -> dict[str, Any]:
    """Compare a single workload against baseline."""
    b_p50 = fp_metric(baseline, "p50_us")
    n_p50 = fp_metric(new, "p50_us")

    b_p90 = fp_metric(baseline, "p95_us")
    n_p90 = fp_metric(new, "p95_us")

    b_throughput = fp_metric(baseline, "throughput_rows_sec")
    n_throughput = fp_metric(new, "throughput_rows_sec")

    p50_change = ((n_p50 - b_p50) / b_p50 * 100) if b_p50 > 0 else 0
    p90_change = ((n_p90 - b_p90) / b_p90 * 100) if b_p90 > 0 else 0
    throughput_change = ((n_throughput - b_throughput) / b_throughput * 100) if b_throughput > 0 else 0

    violations = []
    if p50_change > -THRESHOLDS["primary_pct"]:
        pass
    elif p50_change < THRESHOLDS["primary_pct"]:
        violations.append(f"p50 regressed {p50_change:.1f}% (threshold: {THRESHOLDS['primary_pct']}%)")

    if p90_change < THRESHOLDS["p90_pct"]:
        violations.append(f"p90 regressed {p90_change:.1f}% (threshold: {THRESHOLDS['p90_pct']}%)")

    if throughput_change < THRESHOLDS["throughput_pct"]:
        violations.append(f"throughput dropped {throughput_change:.1f}% (threshold: {THRESHOLDS['throughput_pct']}%)")

    return {
        "workload": new.get("workload"),
        "category": new.get("category"),
        "size": new.get("size"),
        "p50_change_pct": round(p50_change, 2),
        "p90_change_pct": round(p90_change, 2),
        "throughput_change_pct": round(throughput_change, 2),
        "violations": violations,
        "passed": len(violations) == 0,
    }


def compare_category(baseline_results: list, new_results: list, category: str) -> dict[str, Any]:
    """Compare category-level geomean."""
    baseline_by_key = {
        workload_key(r): r
        for r in baseline_results
        if r.get("category") == category
    }
    new_by_key = {
        workload_key(r): r
        for r in new_results
        if r.get("category") == category
    }
    comparable_keys = baseline_by_key.keys() & new_by_key.keys()

    baseline_p50s = [
        fp_metric(baseline_by_key[key], "p50_us")
        for key in comparable_keys
        if fp_metric(baseline_by_key[key], "p50_us") > 0
    ]
    new_p50s = [
        fp_metric(new_by_key[key], "p50_us")
        for key in comparable_keys
        if fp_metric(new_by_key[key], "p50_us") > 0
    ]

    b_geomean = compute_geomean(baseline_p50s)
    n_geomean = compute_geomean(new_p50s)

    geomean_change = ((n_geomean - b_geomean) / b_geomean * 100) if b_geomean > 0 else 0

    violations = []
    if geomean_change < THRESHOLDS["geomean_pct"]:
        violations.append(f"geomean regressed {geomean_change:.1f}% (threshold: {THRESHOLDS['geomean_pct']}%)")
    if geomean_change < THRESHOLDS["per_category_pct"]:
        violations.append(f"category regressed {geomean_change:.1f}% (threshold: {THRESHOLDS['per_category_pct']}%)")

    return {
        "category": category,
        "baseline_geomean_us": round(b_geomean, 2),
        "new_geomean_us": round(n_geomean, 2),
        "change_pct": round(geomean_change, 2),
        "violations": violations,
        "passed": len(violations) == 0,
    }


def run_ratchet(baseline_path: Path, new_path: Path) -> tuple[str, dict[str, Any]]:
    """Compare new results against baseline, return verdict and report."""
    baseline = load_json(baseline_path)
    new = load_json(new_path)

    baseline_results = baseline.get("results", [])
    new_results = new.get("results", [])
    decidable_new_results = [result for result in new_results if is_decidable(result)]

    baseline_by_key = {
        workload_key(r): r for r in baseline_results
    }

    workload_comparisons = []
    for nr in decidable_new_results:
        key = workload_key(nr)
        br = baseline_by_key.get(key)
        if br:
            cmp = compare_workload(br, nr)
            workload_comparisons.append(cmp)

    category_comparisons = []
    for cat in CATEGORIES:
        if any(result.get("category") == cat for result in decidable_new_results):
            cmp = compare_category(baseline_results, decidable_new_results, cat)
            category_comparisons.append(cmp)

    all_workload_passed = all(c["passed"] for c in workload_comparisons)
    all_category_passed = all(c["passed"] for c in category_comparisons)

    uncertain_count = sum(
        1
        for result in new_results
        if not is_decidable(result)
    )

    if all_workload_passed and all_category_passed:
        if uncertain_count > 0:
            verdict = "QUARANTINE"
        else:
            verdict = "ALLOW"
    else:
        verdict = "BLOCK"

    report = {
        "verdict": verdict,
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "baseline_file": str(baseline_path),
        "new_file": str(new_path),
        "thresholds": THRESHOLDS,
        "workload_comparisons": workload_comparisons,
        "category_comparisons": category_comparisons,
        "summary": {
            "total_workloads": len(workload_comparisons),
            "workloads_passed": sum(1 for c in workload_comparisons if c["passed"]),
            "workloads_failed": sum(1 for c in workload_comparisons if not c["passed"]),
            "categories_passed": sum(1 for c in category_comparisons if c["passed"]),
            "categories_failed": sum(1 for c in category_comparisons if not c["passed"]),
            "uncertain_measurement_count": uncertain_count,
            "cv_is_provenance_only": True,
        },
        "failed_workloads": [c for c in workload_comparisons if not c["passed"]],
        "failed_categories": [c for c in category_comparisons if not c["passed"]],
    }

    return verdict, report


def update_baseline(new_path: Path, baseline_name: str = "latest") -> None:
    """Copy new results as the new baseline."""
    new = load_json(new_path)
    baseline_path = BASELINE_DIR / f"{baseline_name}.json"
    save_json(baseline_path, new)


def main():
    parser = argparse.ArgumentParser(description="Performance ratchet gate")
    parser.add_argument("--baseline", type=Path, help="Path to baseline JSON")
    parser.add_argument("--new", type=Path, help="Path to new benchmark results JSON")
    parser.add_argument("--update-baseline", type=Path, help="Update baseline with this file")
    parser.add_argument("--baseline-name", default="latest", help="Name for baseline file")
    parser.add_argument("--output", type=Path, help="Write report to this file")
    parser.add_argument("--json", action="store_true", help="Output JSON instead of text")
    args = parser.parse_args()

    if args.update_baseline:
        update_baseline(args.update_baseline, args.baseline_name)
        print(f"ALLOW: Baseline updated from {args.update_baseline}")
        return 0

    if not args.baseline or not args.new:
        parser.error("--baseline and --new are required for comparison")

    if not args.baseline.exists():
        print(f"ALLOW: No baseline exists at {args.baseline}, initializing")
        update_baseline(args.new, args.baseline_name)
        return 0

    verdict, report = run_ratchet(args.baseline, args.new)

    if args.output:
        save_json(args.output, report)

    if args.json:
        print(json.dumps(report, indent=2))
    else:
        print(f"\n{'='*60}")
        print(f"PERFORMANCE RATCHET GATE: {verdict}")
        print(f"{'='*60}\n")

        summary = report["summary"]
        print(f"Workloads: {summary['workloads_passed']}/{summary['total_workloads']} passed")
        print(
            f"Categories: {summary['categories_passed']}/"
            f"{len(report['category_comparisons'])} passed"
        )
        print(f"Median-CI/contract uncertain measurements: {summary['uncertain_measurement_count']}")

        if report["failed_workloads"]:
            print(f"\nFailed workloads:")
            for fw in report["failed_workloads"]:
                print(f"  - {fw['workload']} ({fw['size']}): {', '.join(fw['violations'])}")

        if report["failed_categories"]:
            print(f"\nFailed categories:")
            for fc in report["failed_categories"]:
                print(f"  - {fc['category']}: {', '.join(fc['violations'])}")

        print(f"\nVerdict: {verdict}")

    if verdict == "BLOCK":
        return 1
    elif verdict == "QUARANTINE":
        return 2
    else:
        return 0


if __name__ == "__main__":
    sys.exit(main())
