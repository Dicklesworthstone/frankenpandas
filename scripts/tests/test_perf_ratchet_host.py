#!/usr/bin/env python3
"""Worker-comparability gate on the perf ratchet (br-frankenpandas-s7x8z).

THE OBSERVED DEFECT CLASS, measured by the fleet on 2026-08-15 and not inferred:
frankenscipy ran the SAME cubic `splu` cell on two different rch workers and got
**1.2693x** on one and **0.0093x** on the other — a 13.6x swing — with **both
A/A nulls PASSING**. An A/A null controls WITHIN-invocation noise; it says
nothing about BETWEEN-worker differences in CPU model, cache, memory bandwidth
or contention. `scripts/perf_ratchet.py` had no host awareness at all, so it
would compare a candidate against a baseline taken on another machine and call
the machine difference a code regression (or let a real one hide).

Three distinct hosts already coexist in this repo's banked rows, so this is
live rather than hypothetical: thinkstation1 (64-thread Threadripper PRO
5975WX), frankenlibc-test (10-thread EPYC VM), and vmi1149989.

THE GATE THIS ENFORCES: `scripts/apply_ratchet.sh` propagates this script's exit
code, so 2 (QUARANTINE) stops a cross-worker ratchet from being read as a pass
or a fail.

Run:  python3 -m pytest scripts/tests/ -v
      python3 scripts/tests/test_perf_ratchet_host.py    (no pytest needed)
"""
from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from perf_ratchet import (  # noqa: E402
    comparability_identity,
    host_comparability,
    run_ratchet,
    scope_annotation,
)

THINKSTATION = {
    "host_identity": "thinkstation1",
    "cpu_model": "AMD Ryzen Threadripper PRO 5975WX 32-Cores",
    "physical_cores": 32,
    "logical_threads": 64,
    "threads_per_core": 2,
    "affinity_logical_cpu_cap": 64,
    "cpu_governors": ["performance"],
    "runtime_detected_isa_features": ["sse2", "avx", "avx2", "fma"],
}

EPYC_VM = {
    "host_identity": "frankenlibc-test",
    "cpu_model": "AMD EPYC Processor (with IBPB)",
    "physical_cores": 10,
    "logical_threads": 10,
    "threads_per_core": 1,
    "affinity_logical_cpu_cap": 10,
    "cpu_governors": [],
    "runtime_detected_isa_features": ["sse2", "avx", "avx2", "fma"],
}


# Two distinct harnesses, the shape frankenlibc measured a ~2x spread between on
# ONE worker. Only the sha256 is identity; `path` deliberately varies below
# because the same harness is banked in this repo under both /opt/fpbench/... and
# /data/projects/... (br-frankenpandas-s7x8z).
HARNESS_A = {
    "sha256": "f0a5cef146a089144075b01fb07a73bec78b681bc171fe51cd622a3f75dc7d9e",
    "bytes": 142435,
    "path": "/opt/fpbench/vs_pandas_harness.py",
}
HARNESS_B = {
    "sha256": "cc98cbe5d187b262764114bac407553ed07aecd87de7aa1041e1e9a5b85a371e",
    "bytes": 63051,
    "path": "/data/projects/frankenpandas/benches/vs_pandas_harness.py",
}


def _row(p50: float) -> dict:
    return {
        "workload": "groupby_sum",
        "category": "groupby",
        "size": "100k",
        "dtype": "int64",
        "verdict": "DECIDED",
        "frankenpandas": {"p50_us": p50, "p95_us": p50 * 1.2, "throughput_rows_sec": 1e6},
    }


def _doc(p50: float, fingerprint: dict | None, harness: dict | None = HARNESS_A) -> dict:
    doc: dict = {"results": [_row(p50)]}
    if fingerprint is not None:
        doc["host_fingerprint"] = fingerprint
    if harness is not None:
        doc["harness_source"] = harness
    return doc


def _ratchet(baseline: dict, candidate: dict) -> tuple[str, dict]:
    with tempfile.TemporaryDirectory() as tmp:
        base_path = Path(tmp) / "baseline.json"
        new_path = Path(tmp) / "new.json"
        base_path.write_text(json.dumps(baseline))
        new_path.write_text(json.dumps(candidate))
        return run_ratchet(base_path, new_path)


def test_same_worker_still_ratchets_normally():
    """CONTROL. Without this, "always QUARANTINE" passes every other test here."""
    verdict, report = _ratchet(_doc(100.0, THINKSTATION), _doc(101.0, THINKSTATION))
    assert verdict == "ALLOW", report
    assert report["host_comparability"]["comparable"] is True
    assert report["summary"]["total_workloads"] == 1


def test_same_worker_still_blocks_a_real_regression():
    """CONTROL. The gate must not have become a way to stop detecting anything."""
    verdict, report = _ratchet(_doc(100.0, THINKSTATION), _doc(150.0, THINKSTATION))
    assert verdict == "BLOCK", report
    assert report["summary"]["workloads_failed"] == 1


def test_different_worker_refuses_instead_of_blocking():
    """The headline case: a 50% "regression" that is really a machine change."""
    verdict, report = _ratchet(_doc(100.0, THINKSTATION), _doc(150.0, EPYC_VM))
    assert verdict == "QUARANTINE", report
    assert report["host_comparability"]["comparable"] is False
    # Refused, not merely down-weighted: nothing was compared.
    assert report["workload_comparisons"] == []
    assert report["category_comparisons"] == []
    assert report["summary"]["refused_reason"] == "host_not_comparable"


def test_different_worker_refuses_an_apparent_IMPROVEMENT_too():
    """The direction that would otherwise bank a free win.

    A cross-worker comparison can manufacture a pass as easily as a fail — the
    splu swing went both ways — so refusing only regressions would let the
    faster machine launder a speedup into the baseline.
    """
    verdict, report = _ratchet(_doc(150.0, THINKSTATION), _doc(100.0, EPYC_VM))
    assert verdict == "QUARANTINE", report
    assert report["host_comparability"]["comparable"] is False


def test_missing_fingerprint_is_not_a_wildcard():
    """144/165 artifacts/bench rows predate host_fingerprint.

    A run that names no host must NOT match everything. This is the arm that
    fails a `baseline_host == candidate_host` implementation written with
    `None == None`, which would silently declare two unplaceable runs
    comparable — the single most likely wrong version of this fix, since 1057
    of 1058 tests/artifacts/perf rows are exactly that shape.
    """
    for baseline_fp, candidate_fp in (
        (None, THINKSTATION),
        (THINKSTATION, None),
        (None, None),
    ):
        verdict, report = _ratchet(_doc(100.0, baseline_fp), _doc(101.0, candidate_fp))
        assert verdict == "QUARANTINE", (baseline_fp, candidate_fp, report)
        assert report["host_comparability"]["comparable"] is False


def test_hostname_alone_does_not_certify_comparability():
    """Same name, different machine underneath.

    Fails an implementation that compares only `host_identity` — which is the
    cheapest possible version of this fix and is wrong, because a reused VM name
    can front a different CPU or a different thread budget.
    """
    renamed = dict(EPYC_VM)
    renamed["host_identity"] = THINKSTATION["host_identity"]
    verdict, report = _ratchet(_doc(100.0, THINKSTATION), _doc(101.0, renamed))
    assert verdict == "QUARANTINE", report
    differing = report["host_comparability"]["reason"]
    assert "cpu_model" in differing and "logical_threads" in differing


def test_thread_budget_alone_breaks_comparability():
    """Same box, but the candidate was allowed fewer CPUs.

    A parallel arm's ratio scales with the thread budget, so an affinity cap
    change is a machine change even on one host.
    """
    capped = dict(THINKSTATION)
    capped["affinity_logical_cpu_cap"] = 8
    verdict, report = _ratchet(_doc(100.0, THINKSTATION), _doc(101.0, capped))
    assert verdict == "QUARANTINE", report
    assert "affinity_logical_cpu_cap" in report["host_comparability"]["reason"]


def test_isa_feature_order_is_not_a_machine_difference():
    """Enumeration order must not manufacture a refusal.

    Without this, a gate that compares the raw lists quarantines every run
    against itself the moment the harness reorders its feature probe — a false
    refusal is as bad as a false pass, because it trains the reader to ignore
    QUARANTINE.
    """
    reordered = dict(THINKSTATION)
    reordered["runtime_detected_isa_features"] = ["fma", "avx2", "sse2", "avx"]
    assert comparability_identity(_doc(1.0, THINKSTATION)) == comparability_identity(
        _doc(1.0, reordered)
    )
    verdict, _ = _ratchet(_doc(100.0, THINKSTATION), _doc(101.0, reordered))
    assert verdict == "ALLOW"


def test_governor_is_provenance_not_a_gate():
    """frankenfs measured that a load-style veto does not predict the ratio
    (load varied 4.9x, ratio spread 6.46%, r=-0.35), so gating on it rejects
    runs on a signal uncorrelated with error. Governor stays provenance."""
    other_governor = dict(THINKSTATION)
    other_governor["cpu_governors"] = ["schedutil"]
    verdict, _ = _ratchet(_doc(100.0, THINKSTATION), _doc(101.0, other_governor))
    assert verdict == "ALLOW"


def test_same_worker_different_harness_refuses():
    """THE frankenlibc CASE, and the reason worker identity alone is not enough.

    MEASURED BY THE FLEET 2026-08-15: malloc/free on ONE worker (hz2) under two
    separately-sanctioned harnesses read 5.9459x and 12.385414x — a ~2x spread —
    with BOTH A/A nulls passing in tolerance. Same machine, same primitive,
    different instrument, and the null caught none of it.

    Fails any implementation that keys comparability on the host alone, which is
    what this gate did before the harness half was added.
    """
    verdict, report = _ratchet(
        _doc(100.0, THINKSTATION, HARNESS_A), _doc(150.0, THINKSTATION, HARNESS_B)
    )
    assert verdict == "QUARANTINE", report
    assert report["host_comparability"]["comparable"] is False
    # The refusal must SAY it was the harness, not report an empty diff.
    assert "harness_sha256" in report["host_comparability"]["reason"], report


def test_missing_harness_is_not_a_wildcard():
    """1057/1058 tests/artifacts/perf rows name no harness at all.

    A run that cannot name its instrument is uncomparable for the same reason a
    run that cannot name its machine is — otherwise `None == None` would declare
    two unattributable runs comparable.
    """
    for baseline_h, candidate_h in ((None, HARNESS_A), (HARNESS_A, None), (None, None)):
        verdict, report = _ratchet(
            _doc(100.0, THINKSTATION, baseline_h), _doc(101.0, THINKSTATION, candidate_h)
        )
        assert verdict == "QUARANTINE", (baseline_h, candidate_h, report)
        assert report["host_comparability"]["comparable"] is False


def test_same_harness_staged_at_a_different_path_still_compares():
    """Only the sha256 is identity; the path is not.

    This repo banks the SAME harness (f0a5cef1…) as both /opt/fpbench/… and
    /data/projects/…, so keying on the path would refuse a run against itself
    purely for having been staged elsewhere on the worker — a false refusal,
    which trains the reader to ignore QUARANTINE just as surely as a false pass.
    """
    relocated = dict(HARNESS_A)
    relocated["path"] = "/data/projects/frankenpandas/benches/vs_pandas_harness.py"
    verdict, report = _ratchet(
        _doc(100.0, THINKSTATION, HARNESS_A), _doc(101.0, THINKSTATION, relocated)
    )
    assert verdict == "ALLOW", report


def test_scope_annotation_agrees_with_the_gate_that_refuses():
    """The retro-flag must never contradict the gate.

    `scope_annotation` is derived from the SAME `comparability_identity` the
    ratchet refuses on, so an artifact can never be stamped "comparable" while
    the gate would quarantine it. That is the whole reason the flagger lives in
    this file instead of being a one-off script: two implementations of
    "comparable" would drift, and the stamped one is the one humans would quote.
    """
    placed = _doc(100.0, THINKSTATION, HARNESS_A)
    annotation = scope_annotation(placed)
    assert annotation["status"] == "comparable"
    assert annotation["names_worker"] and annotation["names_harness"]
    assert annotation["identity"] == comparability_identity(placed)

    for fingerprint, harness in ((None, HARNESS_A), (THINKSTATION, None), (None, None)):
        unplaced = _doc(100.0, fingerprint, harness)
        annotation = scope_annotation(unplaced)
        assert annotation["status"] == "worker_scoped_unknown", (fingerprint, harness)
        assert annotation["identity"] is None
        assert comparability_identity(unplaced) is None
        # It must say WHICH half is missing, or the flag is not actionable.
        if fingerprint is None:
            assert "worker" in annotation["note"]
        if harness is None:
            assert "harness" in annotation["note"]


def test_scope_annotation_is_idempotent():
    """Purely a function of the document — no timestamp, so re-flagging the
    corpus produces no diff churn and cannot be mistaken for new evidence."""
    doc = _doc(100.0, THINKSTATION, HARNESS_A)
    assert scope_annotation(doc) == scope_annotation(doc)


def test_comparability_is_symmetric():
    forward = host_comparability(_doc(1.0, THINKSTATION), _doc(1.0, EPYC_VM))
    backward = host_comparability(_doc(1.0, EPYC_VM), _doc(1.0, THINKSTATION))
    assert forward["comparable"] is backward["comparable"] is False


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
