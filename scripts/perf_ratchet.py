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
  - QUARANTINE: Some measurements are contract-invalid or median-CI undecidable,
                OR baseline and candidate cannot be placed on the same worker

CV is retained as provenance only. It never decides a ratchet verdict.

WORKER COMPARABILITY (br-frankenpandas-s7x8z)
---------------------------------------------
A ratchet compares two runs. If they did not execute on the same worker, the
difference between them is not a code change — it is a machine change, and this
gate would automate the exact hazard the fleet measured on 2026-08-15:

  frankenscipy ran the SAME cubic splu cell on two rch workers and got 1.2693x
  and 0.0093x — a 13.6x swing — with BOTH A/A nulls PASSING. An A/A null
  controls WITHIN-invocation noise; it says nothing about BETWEEN-worker
  differences in CPU model, cache, memory bandwidth, or contention.

That is not hypothetical here: three distinct hosts already coexist in this
repo's banked rows (thinkstation1, a 64-thread Threadripper PRO 5975WX;
frankenlibc-test, a 10-thread EPYC VM; vmi1149989).

So this gate REFUSES rather than answers when the two runs cannot be placed on
one worker. "Not comparable" is a third answer, distinct from "no regression"
and from "regression", and it is the honest one — a cross-worker comparison can
manufacture a BLOCK or hide one with equal ease, so the workload and category
comparisons are not computed at all in that case.

A run that names no host at all is uncomparable for the same reason: 144 of 165
artifacts/bench rows and 1057 of 1058 tests/artifacts/perf rows predate the
`host_fingerprint` block. Those are honest raw measurements; they simply cannot
be placed on a machine, so they cannot ratchet against anything.

There is deliberately NO --allow-cross-host flag. The remedy for a quarantine
here is to re-measure the candidate on the baseline's worker (or re-baseline on
the candidate's), not to wave the check through.

DELETION CONDITION: this check can go when every row in .bench-history carries a
`host_fingerprint` AND the ratchet selects its baseline per-worker, at which
point a mismatch is unreachable rather than merely refused.

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

# The host_fingerprint fields that decide whether two runs are comparable.
#
# Chosen as the ones that change the RATIO, not merely the machine's name:
# cpu_model and the ISA feature set change which kernels run at all; the thread
# counts change how far a parallel arm scales; the affinity cap changes how much
# of the box the process was allowed to use. host_identity is included so two
# same-spec VMs on different physical hosts still refuse to be compared — the
# splu swing was between workers of the same family.
#
# Governor is deliberately NOT here: frankenfs measured that an external-load
# style veto metric does not predict the ratio (load varied 4.9x, ratio spread
# 6.46%, r=-0.35), so gating on it would reject runs on a signal uncorrelated
# with error. Governor stays provenance, like CV. (br-frankenpandas-s7x8z)
COMPARABILITY_FIELDS = (
    "host_identity",
    "cpu_model",
    "logical_threads",
    "threads_per_core",
    "affinity_logical_cpu_cap",
    "runtime_detected_isa_features",
)


def comparability_identity(doc: dict[str, Any]) -> dict[str, Any] | None:
    """The facts two runs must agree on, or None if the run cannot be placed.

    None and a populated dict are BOTH refusals when they disagree; the caller
    must not treat a missing fingerprint as a wildcard that matches anything.

    THE HARNESS IS PART OF THE IDENTITY, not just the machine
    (br-frankenpandas-s7x8z). MEASURED BY THE FLEET 2026-08-15: frankenlibc ran
    malloc/free on ONE worker (hz2) under two separately-sanctioned harnesses and
    got 5.9459x and 12.385414x — a ~2x spread — with BOTH A/A nulls passing in
    tolerance. So a passing null certifies neither the machine nor the
    instrument, and a same-worker comparison across two harnesses is exactly as
    meaningless as a cross-worker one.

    That hazard is live in this repo, not imported: `artifacts/bench` holds
    18 DISTINCT `harness_source.sha256` values, and `thinkstation1` alone carries
    rows from three different harnesses.

    Only the harness's `sha256` counts, never its `path`: the same harness
    (f0a5cef1…) appears banked as both `/opt/fpbench/…` and `/data/projects/…`,
    so keying on the path would refuse a run against itself purely for having
    been staged somewhere else on the worker.
    """
    fingerprint = doc.get("host_fingerprint")
    if not isinstance(fingerprint, dict):
        return None
    if not fingerprint.get("host_identity"):
        return None

    harness = doc.get("harness_source")
    harness_sha = harness.get("sha256") if isinstance(harness, dict) else None
    if not harness_sha:
        return None

    identity: dict[str, Any] = {"harness_sha256": str(harness_sha)}
    for field in COMPARABILITY_FIELDS:
        value = fingerprint.get(field)
        # Order within the ISA set is an enumeration detail, not a machine
        # difference, so compare it as a set rendered canonically.
        if isinstance(value, list):
            value = sorted(str(item) for item in value)
        identity[field] = value
    return identity


def describe_host(identity: dict[str, Any] | None) -> str:
    if identity is None:
        return "unknown (run names no host_fingerprint.host_identity or no harness_source.sha256)"
    return (
        f"{identity.get('host_identity')} "
        f"[{identity.get('cpu_model')}, "
        f"{identity.get('logical_threads')} logical threads] "
        f"under harness {str(identity.get('harness_sha256'))[:12]}"
    )


def host_comparability(baseline: dict[str, Any], new: dict[str, Any]) -> dict[str, Any]:
    """Decide whether these two runs may be ratcheted against each other."""
    baseline_identity = comparability_identity(baseline)
    new_identity = comparability_identity(new)

    if baseline_identity is None or new_identity is None:
        unplaceable = [
            side
            for side, identity in (("baseline", baseline_identity), ("candidate", new_identity))
            if identity is None
        ]
        reason = (
            f"{' and '.join(unplaceable)} names no worker and/or no harness, so the two "
            f"runs cannot be shown to have measured the same thing on the same machine"
        )
        comparable = False
    elif baseline_identity != new_identity:
        # Iterate the IDENTITY's own keys, not COMPARABILITY_FIELDS — the latter
        # is only the host half, so a pure HARNESS mismatch (the frankenlibc
        # 5.9459x vs 12.385414x shape) would have reported an empty differing
        # list and read as an unexplained refusal. (br-frankenpandas-s7x8z)
        differing = sorted(
            field
            for field in set(baseline_identity) | set(new_identity)
            if baseline_identity.get(field) != new_identity.get(field)
        )
        reason = (
            f"baseline ran on {describe_host(baseline_identity)} and candidate ran on "
            f"{describe_host(new_identity)}; differing: {', '.join(differing)}"
        )
        comparable = False
    else:
        reason = f"both runs on {describe_host(new_identity)}"
        comparable = True

    return {
        "comparable": comparable,
        "reason": reason,
        "baseline_host": baseline_identity,
        "candidate_host": new_identity,
        "remedy": (
            None
            if comparable
            else "re-measure the candidate on the baseline's worker WITH the baseline's "
            "harness, or re-baseline on the candidate's; do not compare across workers "
            "or across harnesses"
        ),
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

    # DIRECTION (br-frankenpandas-a9fh8). p50_us and p95_us are LATENCY: a
    # POSITIVE change is slower, i.e. the regression. throughput_rows_sec is a
    # RATE: a NEGATIVE change is the regression. The two conventions are
    # opposite, and the latency pair used to be tested with the rate one:
    #
    #     if p50_change > -THRESHOLDS["primary_pct"]:   # +50% slower > +3%
    #         pass                                      # ...swallowed
    #     elif p50_change < THRESHOLDS["primary_pct"]:  # fires at -3%, a WIN
    #
    # so a 50% slowdown returned ALLOW and a 50% speedup returned BLOCK,
    # reported as "p50 regressed -50.0%". THRESHOLDS entries are negative
    # "allowed worsening" budgets, so the latency budget is their negation.
    violations = []
    latency_budget = -THRESHOLDS["primary_pct"]
    if p50_change > latency_budget:
        violations.append(
            f"p50 regressed +{p50_change:.1f}% slower (budget: +{latency_budget:.1f}%)"
        )

    p90_budget = -THRESHOLDS["p90_pct"]
    if p90_change > p90_budget:
        violations.append(
            f"p90 regressed +{p90_change:.1f}% slower (budget: +{p90_budget:.1f}%)"
        )

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

    # Latency direction, same inversion as compare_workload — the geomean is
    # over p50_us, so POSITIVE is slower. (br-frankenpandas-a9fh8)
    violations = []
    geomean_budget = -THRESHOLDS["geomean_pct"]
    if geomean_change > geomean_budget:
        violations.append(
            f"geomean regressed +{geomean_change:.1f}% slower (budget: +{geomean_budget:.1f}%)"
        )
    category_budget = -THRESHOLDS["per_category_pct"]
    if geomean_change > category_budget:
        violations.append(
            f"category regressed +{geomean_change:.1f}% slower (budget: +{category_budget:.1f}%)"
        )

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

    # Worker comparability decides whether there is a comparison to make AT ALL,
    # so it runs before any threshold. The workload/category comparisons are
    # deliberately left empty on refusal rather than computed-and-ignored: a
    # cross-worker delta is not a weak signal to be down-weighted, it is a
    # measurement of a different machine, and reporting it under a QUARANTINE
    # would invite exactly the "well it only regressed 4%" reading that the
    # 13.6x splu swing refutes. (br-frankenpandas-s7x8z)
    comparability = host_comparability(baseline, new)
    if not comparability["comparable"]:
        report = {
            "verdict": "QUARANTINE",
            "timestamp": datetime.now(timezone.utc).isoformat(),
            "baseline_file": str(baseline_path),
            "new_file": str(new_path),
            "thresholds": THRESHOLDS,
            "host_comparability": comparability,
            "workload_comparisons": [],
            "category_comparisons": [],
            "summary": {
                "total_workloads": 0,
                "workloads_passed": 0,
                "workloads_failed": 0,
                "categories_passed": 0,
                "categories_failed": 0,
                "uncertain_measurement_count": len(new_results),
                "cv_is_provenance_only": True,
                "refused_reason": "host_not_comparable",
            },
            "failed_workloads": [],
            "failed_categories": [],
        }
        return "QUARANTINE", report

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
        "host_comparability": comparability,
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


def scope_annotation(doc: dict[str, Any]) -> dict[str, Any]:
    """The comparability verdict for ONE banked artifact, as a storable block.

    Derived from `comparability_identity`, so the annotation written into an
    artifact can never disagree with the gate that refuses comparisons — the tool
    that DEFINES comparability is the tool that records it. Purely a function of
    the document (no timestamp), so re-running is idempotent and produces no
    diff churn.
    """
    fingerprint = doc.get("host_fingerprint")
    harness = doc.get("harness_source")
    names_worker = bool(
        isinstance(fingerprint, dict) and fingerprint.get("host_identity")
    )
    names_harness = bool(isinstance(harness, dict) and harness.get("sha256"))
    identity = comparability_identity(doc)

    if identity is not None:
        note = (
            "Names both its worker and its harness, so it may be compared to "
            "another row that names the SAME pair."
        )
        status = "comparable"
    else:
        missing = [
            label
            for label, present in (("worker", names_worker), ("harness", names_harness))
            if not present
        ]
        note = (
            f"NOT COMPARABLE TO ANY OTHER ROW: names no {' and no '.join(missing)}. "
            "The measurement itself is honest and is neither deleted nor "
            "regenerated; it simply cannot be placed, so no ratio may be drawn "
            "between it and another row."
        )
        status = "worker_scoped_unknown"

    return {
        "status": status,
        "names_worker": names_worker,
        "names_harness": names_harness,
        "identity": identity,
        "note": note,
        "bead": "br-frankenpandas-s7x8z",
    }


def flag_scope(paths: list[Path], apply: bool) -> dict[str, int]:
    """Annotate banked ratio artifacts with their comparability scope.

    Retro-flagging, per the 2026-08-15 fleet directive and this bead's gap 1:
    a row whose two arms cannot be shown to have run on the same machine under
    the same harness is marked `worker_scoped_unknown` rather than silently
    trusted. Nothing is deleted and nothing is regenerated.
    """
    counts: dict[str, int] = {
        "comparable": 0,
        "worker_scoped_unknown": 0,
        "unchanged": 0,
        "unparseable": 0,
        "foreign_schema": 0,
    }
    for path in paths:
        try:
            doc = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            counts["unparseable"] += 1
            continue
        # A WELL-FORMED ARTIFACT IN A DIFFERENT SCHEMA IS NOT AN ERROR, AND
        # MUST NOT HIDE IN THE ERROR BUCKET. `unparseable` used to absorb both
        # malformed JSON and every valid banked row that simply is not a
        # harness results document — raw hyperfine dumps (`{"times_us": ...}`),
        # samply profiles, csv round-trip checksums. Measured over
        # tests/artifacts/perf: 2 files are genuinely bad JSON and 65 are
        # foreign-schema, so a single count of 67 reads as "67 broken files"
        # when it is really "65 rows this retro-flag silently skipped".
        #
        # They are the rows LEAST able to name a worker — none of those shapes
        # carries a host_fingerprint at all — so counting them as errors is
        # exactly the shape where an incomplete sweep reads as a finished one.
        # Split so the skipped class is visible in the summary.
        # (br-frankenpandas-s7x8z gap 1)
        if not isinstance(doc, dict):
            counts["unparseable"] += 1
            continue
        if "results" not in doc:
            counts["foreign_schema"] += 1
            continue
        annotation = scope_annotation(doc)
        counts[annotation["status"]] += 1
        if doc.get("comparability_scope") == annotation:
            counts["unchanged"] += 1
            continue
        if apply:
            doc["comparability_scope"] = annotation
            path.write_text(json.dumps(doc, indent=2) + "\n", encoding="utf-8")
    return counts


def update_baseline(new_path: Path, baseline_name: str = "latest") -> None:
    """Copy new results as the new baseline."""
    new = load_json(new_path)
    # A baseline that names no worker quarantines every future comparison
    # against it, so say so at the moment it is banked rather than leaving the
    # next run to discover it. (br-frankenpandas-s7x8z)
    identity = comparability_identity(new)
    if identity is None:
        print(
            f"WARNING: {new_path} carries no host_fingerprint.host_identity. "
            f"It is being banked as a baseline anyway, but every ratchet run "
            f"against it will QUARANTINE as host-not-comparable until it is "
            f"re-measured with the sanctioned harness."
        )
    else:
        print(f"Baseline worker: {describe_host(identity)}")
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
    parser.add_argument(
        "--flag-scope",
        type=Path,
        help="Annotate every banked ratio artifact under this directory with its "
        "comparability scope (worker + harness). Reports without --apply-scope.",
    )
    parser.add_argument(
        "--apply-scope",
        action="store_true",
        help="Write the --flag-scope annotations instead of only reporting them.",
    )
    args = parser.parse_args()

    if args.flag_scope:
        paths = sorted(args.flag_scope.glob("*.json"))
        counts = flag_scope(paths, args.apply_scope)
        verb = "annotated" if args.apply_scope else "would annotate"
        print(f"{args.flag_scope}: {len(paths)} artifact(s)")
        print(f"  comparable (names worker AND harness): {counts['comparable']}")
        print(f"  worker_scoped_unknown                : {counts['worker_scoped_unknown']}")
        print(f"  already current (no rewrite)         : {counts['unchanged']}")
        print(f"  malformed JSON (not annotatable)     : {counts['unparseable']}")
        print(
            f"  SKIPPED, foreign schema (no 'results'): {counts['foreign_schema']}"
            "   <-- banked rows this sweep did NOT flag"
        )
        print(f"  {verb}: {counts['comparable'] + counts['worker_scoped_unknown']}")
        return 0

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

        comparability = report.get("host_comparability", {})
        if not comparability.get("comparable", True):
            print("REFUSED: baseline and candidate are not comparable (worker + harness).")
            print(f"  {comparability.get('reason')}")
            print(f"  Remedy: {comparability.get('remedy')}")
            print(
                "\n  No workload or category comparison was computed. A cross-worker delta"
                "\n  measures a different machine and a cross-harness delta measures a"
                "\n  different instrument — neither measures a code change."
                "\n  (br-frankenpandas-s7x8z)"
            )
            print(f"\nVerdict: {verdict}")
            return 2
        print(f"Worker: {comparability.get('reason')}")

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
