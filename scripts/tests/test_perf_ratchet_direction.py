#!/usr/bin/env python3
"""Regression direction on the perf ratchet (br-frankenpandas-a9fh8).

THE OBSERVED DEFECT: `scripts/perf_ratchet.py` tested LATENCY with the
THROUGHPUT convention, so the gate was inverted on every latency metric.
Reproduced by calling `compare_workload` directly, no benchmark required:

    p50 100us -> 150us  (50% SLOWER)  -> violations [], passed=True, ALLOW
    p50 100us ->  50us  (50% FASTER)  -> BLOCK, "p50 regressed -50.0%"
    p50 100us ->  96us  (4% faster)   -> BLOCK, "p50 regressed -4.0%"

The gate in `scripts/apply_ratchet.sh` propagates this script's exit code
(0 ALLOW / 1 BLOCK / 2 QUARANTINE), so for as long as that held, every latency
regression passed and only improvements were blocked.

WHY THIS FILE EXISTS AND WHAT IT MUST KEEP DOING: the bug survived because the
metrics were only ever exercised in ONE direction. So every metric here is
asserted in BOTH directions — improve and regress — plus inside the budget.
A test that only checks "a regression blocks" cannot catch a sign error, and a
test that only checks "an improvement allows" cannot either. Only the pair can.

Run:  python3 -m pytest scripts/tests/ -v
      python3 scripts/tests/test_perf_ratchet_direction.py    (no pytest needed)
"""
from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from perf_ratchet import THRESHOLDS, compare_category, compare_workload  # noqa: E402


def _row(p50: float = 100.0, p95: float = 200.0, throughput: float = 1_000_000.0) -> dict:
    return {
        "workload": "groupby_sum",
        "category": "groupby",
        "size": "100k",
        "dtype": "int64",
        "verdict": "DECIDED",
        "frankenpandas": {
            "p50_us": p50,
            "p95_us": p95,
            "throughput_rows_sec": throughput,
        },
    }


def _violations(baseline: dict, new: dict) -> list[str]:
    return compare_workload(baseline, new)["violations"]


def test_p50_slower_is_a_regression():
    """THE HEADLINE. 50% slower must BLOCK; it used to return ALLOW."""
    result = compare_workload(_row(p50=100.0), _row(p50=150.0))
    assert result["passed"] is False, result
    assert any("p50" in v for v in result["violations"]), result


def test_p50_faster_is_not_a_regression():
    """The other half of the pair. 50% faster must NOT be reported as a
    regression; it used to BLOCK with 'p50 regressed -50.0%'."""
    result = compare_workload(_row(p50=100.0), _row(p50=50.0))
    assert result["passed"] is True, result
    assert result["violations"] == [], result


def test_p50_inside_the_budget_passes():
    """2% slower is within the -3% budget."""
    assert _violations(_row(p50=100.0), _row(p50=102.0)) == []


def test_p50_just_outside_the_budget_fails():
    """4% slower exceeds the -3% budget. Pins the boundary so a future rewrite
    cannot quietly widen it while still passing the coarse cases above."""
    assert any("p50" in v for v in _violations(_row(p50=100.0), _row(p50=104.0)))


def test_p90_slower_is_a_regression():
    result = compare_workload(_row(p95=200.0), _row(p95=300.0))
    assert result["passed"] is False, result
    assert any("p90" in v for v in result["violations"]), result


def test_p90_faster_is_not_a_regression():
    result = compare_workload(_row(p95=200.0), _row(p95=100.0))
    assert result["passed"] is True, result


def test_throughput_direction_was_already_correct_and_stays_correct():
    """throughput_rows_sec is a RATE: DOWN is the regression. This is the metric
    the latency pair was wrongly copied from, so it is asserted in both
    directions too — the fix must not have flipped it by symmetry."""
    dropped = compare_workload(_row(throughput=1_000_000.0), _row(throughput=800_000.0))
    assert dropped["passed"] is False, dropped
    assert any("throughput" in v for v in dropped["violations"]), dropped

    raised = compare_workload(_row(throughput=1_000_000.0), _row(throughput=1_200_000.0))
    assert raised["passed"] is True, raised


def test_category_geomean_slower_is_a_regression():
    baseline = [_row(p50=100.0)]
    new = [_row(p50=200.0)]
    result = compare_category(baseline, new, "groupby")
    assert result["passed"] is False, result
    assert result["change_pct"] > 0, result


def test_category_geomean_faster_is_not_a_regression():
    baseline = [_row(p50=200.0)]
    new = [_row(p50=100.0)]
    result = compare_category(baseline, new, "groupby")
    assert result["passed"] is True, result


def test_thresholds_are_still_written_as_negative_budgets():
    """The fix negates these, so a later edit flipping them to positives would
    silently invert the gate again. Pin the convention itself."""
    for key in ("primary_pct", "geomean_pct", "per_category_pct", "p90_pct", "throughput_pct"):
        assert THRESHOLDS[key] < 0, f"{key} must stay a negative allowed-worsening budget"


def _main() -> int:
    failures = 0
    for name, fn in sorted(globals().items()):
        if not name.startswith("test_") or not callable(fn):
            continue
        try:
            fn()
            print(f"PASS {name}")
        except AssertionError as exc:
            failures += 1
            print(f"FAIL {name}: {exc}")
    print(f"\n{failures} failure(s)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(_main())
