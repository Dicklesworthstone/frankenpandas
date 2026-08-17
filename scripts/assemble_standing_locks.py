#!/usr/bin/env python3
"""Assemble one perf-ratchet baseline per comparability identity from banked rows.

WHY THIS EXISTS (the four questions section 2 of the standing orders requires
before an artifact may be created):

  * OBSERVED DEFECT CLASS. On 2026-08-17 a sweep of `artifacts/bench/` found 76
    certified FASTER rows -- all three gate clauses passing -- spread over 14
    distinct comparability identities, of which exactly 2 were locked, and those
    two had been locked an hour earlier by this same sweep's author. Meanwhile
    `.bench-history/latest.json` had sat since 2026-05-25 with `"results": []`,
    so `perf_ratchet.py` -- 28KB of careful budget machinery -- had been
    comparing nothing and returning ALLOW for three months. A ratchet with no
    rows is indistinguishable from a ratchet that passes.
  * CONCRETE CONSUMER. Whoever edits `benches/vs_pandas_harness.py`. The harness
    sha is part of `perf_ratchet.comparability_identity`, so EVERY banked
    baseline quarantines the moment the instrument changes, and this repo has
    already accumulated 18 distinct harness shas. Re-baselining is part of the
    cost of touching the harness; this makes that cost one command instead of a
    chore nobody performs.
  * THE GATE IT ENFORCES. None. It writes baselines; `perf_ratchet.py` is the
    gate and remains the only thing that can BLOCK. This script cannot make a
    regression pass -- at worst it writes a baseline nobody compares against.
  * DELETION CONDITION. Delete it when the harness banks its own baseline on a
    successful certified run, at which point standing rows never fall out of
    date and assembling them after the fact is unnecessary.

TWO SELECTION RULES, both chosen conservatively and both arguable:

  1. Where a workload certified more than once within an identity, the SLOWEST
     certified p50 is banked, not the fastest. Banking the best number would let
     ordinary run-to-run noise trip the ratchet's 3% primary budget -- measured
     replicate agreement in this repo is 0.8-2.7%, uncomfortably close to it. A
     lock that cries wolf gets switched off, and a switched-off lock defends
     nothing.
  2. Thread-capped rows are EXCLUDED. A row measured with fewer logical CPUs
     than the host offers answers a different question: `str_startswith_arrow
     @1M` certified at 1.275x on one core and 4.824x on sixty-four, same host,
     same day, no source change. Banking the capped figure would lock in a
     baseline the engine beats by 3.74x for free.

Read-only with respect to measurements: it runs no build, starts no benchmark,
and never edits an artifact. It only assembles what is already on disk.
"""

from __future__ import annotations

import argparse
import collections
import importlib.util
import json
import pathlib
import re
import sys
from typing import Any

REPO = pathlib.Path(__file__).resolve().parent.parent
BENCH_GLOB = "artifacts/bench/*.json"
BASELINE_DIR = REPO / ".bench-history"


def load_ratchet():
    """Import `perf_ratchet` for its identity rule rather than reimplementing it.

    Reimplementing `comparability_identity` here would be the shadow-harness
    mistake this campaign has already recorded: a private copy that drifts from
    the real one and certifies itself. The gate's definition is the definition.
    """
    spec = importlib.util.spec_from_file_location(
        "perf_ratchet", REPO / "scripts" / "perf_ratchet.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def is_certified(result: dict[str, Any]) -> bool:
    """FASTER with every gate clause passing. Anything less is not a standing win."""
    if result.get("verdict") != "FASTER":
        return False
    clauses = (result.get("median_ci_gate") or {}).get("clauses") or {}
    return bool(clauses) and all(clauses.values())


def is_thread_capped(result: dict[str, Any]) -> bool:
    """Did the subject see fewer logical CPUs than the host has? See rule 2."""
    provenance = result.get("thread_provenance") or {}
    host_threads = provenance.get("logical_threads")
    available = provenance.get("runtime_available_parallelism")
    if not isinstance(available, dict) or not isinstance(host_threads, int):
        return False
    subject = available.get("frankenpandas")
    return isinstance(subject, int) and 0 < subject < host_threads


def identity_slug(identity: dict[str, Any]) -> str:
    host = str(identity.get("host_identity") or "unknown-host")
    harness = str(identity.get("harness_sha256") or "unknown")[:12]
    return re.sub(r"[^A-Za-z0-9_.-]", "_", f"standing_{host}_{harness}")


def collect() -> dict[str, dict[str, Any]]:
    ratchet = load_ratchet()
    groups: dict[str, dict[str, Any]] = {}
    for path in sorted(REPO.glob(BENCH_GLOB)):
        try:
            document = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError):
            continue  # a truncated artifact can only cause an under-report
        if not isinstance(document, dict):
            continue
        identity = ratchet.comparability_identity(document)
        if identity is None:
            continue  # unplaceable runs cannot ratchet against anything
        key = json.dumps(identity, sort_keys=True)
        bucket = groups.setdefault(
            key,
            {"identity": identity, "doc": document, "best": {}, "sources": set()},
        )
        for result in document.get("results", []):
            if not isinstance(result, dict) or not is_certified(result):
                continue
            if is_thread_capped(result):
                continue
            row_key = (result.get("workload"), result.get("size"), result.get("dtype"))
            current = bucket["best"].get(row_key)
            p50 = result["frankenpandas"]["p50_us"]
            # Rule 1: keep the SLOWEST certified p50.
            if current is None or p50 > current["frankenpandas"]["p50_us"]:
                bucket["best"][row_key] = result
            bucket["sources"].add(str(path.relative_to(REPO)))
    return {k: v for k, v in groups.items() if v["best"]}


def write_baseline(bucket: dict[str, Any], apply: bool) -> tuple[str, int]:
    rows = sorted(bucket["best"].values(), key=lambda r: -(r.get("ratio") or 0))
    slug = identity_slug(bucket["identity"])
    document = dict(bucket["doc"])
    document["results"] = rows
    document["summary"] = {
        "total_workloads": len(rows),
        "valid_workloads": len(rows),
        "dropped_high_cv": 0,
        "note": (
            f"STANDING-WIN LOCK for comparability identity {slug}. Assembled by "
            f"scripts/assemble_standing_locks.py from {len(bucket['sources'])} "
            "artifact(s) sharing one identity. Where a workload certified more "
            "than once the SLOWEST certified p50 is banked; thread-capped rows "
            "are excluded. NOTHING WAS RE-MEASURED: this is a baseline document "
            "assembled from prior runs, not a single invocation, and must not be "
            "read as one."
        ),
    }
    document["baseline_provenance"] = {
        "assembled_by": "scripts/assemble_standing_locks.py",
        "identity": bucket["identity"],
        "selection_rule": (
            "slowest certified p50 per (workload,size,dtype); thread-capped rows excluded"
        ),
        "source_artifacts": sorted(bucket["sources"]),
        "rows": [
            {
                "workload": r.get("workload"),
                "size": r.get("size"),
                "ratio": r.get("ratio"),
                "fp_p50_us": r["frankenpandas"]["p50_us"],
                "elf_sha256": r["frankenpandas"]["executable"]["sha256"],
                "invocation_id": r.get("invocation_id"),
            }
            for r in rows
        ],
    }
    if apply:
        (BASELINE_DIR / f"{slug}.json").write_text(json.dumps(document, indent=2) + "\n")
    return slug, len(rows)


def untracked_citations(bucket: dict[str, Any]) -> list[str]:
    """Cited artifacts that git does not track.

    br-frankenpandas-4kig1, 2026-08-17: the two artifacts proving the STANDING
    `mod`/`floordiv @10M` wins sat UNTRACKED for hours while the lock citing them
    was committed and being defended every turn. A baseline whose evidence exists
    only on one machine's disk defends nothing the moment that disk is lost -- and
    that disk was at 100% and falling when this was found.

    Cheap and advisory: it prints, it does not refuse. Assembling a lock from a
    run you have not committed yet is a normal intermediate state; shipping one
    and forgetting is the failure.
    """
    import subprocess

    try:
        tracked = set(
            subprocess.run(
                ["git", "ls-files", "artifacts/bench"],
                capture_output=True,
                text=True,
                check=False,
                cwd=REPO,
            ).stdout.split("\n")
        )
    except OSError:
        return []  # no git available: say nothing rather than cry wolf
    return sorted(
        s for s in bucket["sources"] if s and s not in tracked
    )


def live_harness_sha() -> str:
    """The sha256 of the harness AS IT EXISTS RIGHT NOW, not as any run recorded it."""
    import hashlib

    harness = REPO / "benches" / "vs_pandas_harness.py"
    return hashlib.sha256(harness.read_bytes()).hexdigest()


def report_orphans(groups: dict[str, dict[str, Any]]) -> int:
    """Certified rows banked against a harness that is no longer the live one.

    MEASURED 2026-08-17: 3 of 56 unique locked workloads sat on the live harness
    — the lock set looked 56 strong and was 3. A lock banked against a superseded
    harness is not a weaker lock, it is NO lock: `comparability_identity` refuses
    the comparison outright, so a regression in those workloads is silent.

    This exists because the assembler already tells you to "re-measure the rows it
    can no longer place" and never said WHICH. It prints nothing once the debt is
    zero, which is also its deletion condition.
    """
    live = live_harness_sha()[:12]
    best: dict[tuple[Any, Any, str], tuple[float, str]] = {}
    for bucket in groups.values():
        identity = bucket["identity"]
        harness = str(identity.get("harness_sha256") or "")[:12]
        if harness == live:
            continue
        host = str(identity.get("host_identity") or "unknown-host")
        for result in bucket["best"].values():
            key = (result.get("workload"), result.get("size"), host)
            ratio = float(result.get("ratio") or 0.0)
            if key not in best or ratio > best[key][0]:
                best[key] = (ratio, harness)

    if not best:
        print(f"no orphaned locks — every certified row sits on the live harness ({live}).")
        return 0

    print(f"live harness is {live}; {len(best)} unique certified workload(s) are locked elsewhere.")
    print("Re-measuring these in a clean window is what turns them back into defences:\n")
    for (workload, size, host), (ratio, harness) in sorted(best.items(), key=lambda kv: -kv[1][0]):
        print(f"  {ratio:>10.3f}x  {str(workload):<38} @{str(size):<6} {host:<16} harness={harness}")
    big = sum(1 for ratio, _ in best.values() if ratio >= 2.0)
    print(f"\n{big} of {len(best)} are >=2x wins — a regression there is what would actually hurt.")
    return len(best)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--apply",
        action="store_true",
        help="write the baselines (default: report what would be written)",
    )
    parser.add_argument(
        "--min-rows",
        type=int,
        default=1,
        help="only emit identities carrying at least this many certified workloads",
    )
    parser.add_argument(
        "--orphans",
        action="store_true",
        help="list certified workloads locked against a superseded harness, and exit",
    )
    args = parser.parse_args()

    groups = collect()
    if not groups:
        print("no certified rows found — nothing to lock")
        return 0

    if args.orphans:
        report_orphans(groups)
        return 0

    emitted = total = 0
    for bucket in sorted(groups.values(), key=lambda b: -len(b["best"])):
        if len(bucket["best"]) < args.min_rows:
            continue
        slug, count = write_baseline(bucket, args.apply)
        emitted += 1
        total += count
        verb = "wrote" if args.apply else "would write"
        print(f"{verb} {slug}.json  ({count} workloads)")
        loose = untracked_citations(bucket)
        if loose:
            print(
                f"    !! {len(loose)} cited artifact(s) are NOT TRACKED BY GIT. This "
                f"baseline's evidence\n       exists only on this disk. Commit them "
                f"or the lock is undefendable:"
            )
            for path in loose[:5]:
                print(f"         {path}")
            if len(loose) > 5:
                print(f"         ... and {len(loose) - 5} more")
    print(
        f"\n{emitted} baseline(s), {total} locked workloads."
        + ("" if args.apply else "  Re-run with --apply to write them.")
    )
    print(
        "A baseline only defends anything while the harness is unchanged: the "
        "harness sha is part of the identity, so re-run this after every edit to "
        "benches/vs_pandas_harness.py, and re-measure the rows it can no longer place."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
