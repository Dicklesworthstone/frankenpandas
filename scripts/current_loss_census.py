#!/usr/bin/env python3
"""Rank the CURRENT certified vs-pandas losses, and refuse to be fooled by stale rows.

br-frankenpandas-cu22b / worst-loss triage. Picking the next perf target by
scanning `artifacts/bench/*.json` for the smallest ratio is WRONG in two ways
that both cost real work this session:

  1. DE-DUPLICATING BY LANE AFTER SORTING BY RATIO keeps each lane's WORST-EVER
     row instead of its LATEST. That handed me `floor @1M = 0.072x` as "the worst
     ratio on the board" when the per-package `+sse4.1` adoption (d470a8512,
     2026-08-18) had already made it a certified 1.332x WIN two days later.

  2. A ROW IS ONLY MEANINGFUL WITH ITS ELF. `sqrt @1k` has a correctly certified
     SLOWER 0.165x row -- clean A/A nulls on both arms, load 11.8, equal thread
     counts -- in which FrankenPandas took 188us at n=1000. Every other build
     takes 1.26us and the lane measures 24-26x FASTER. That row is not wrong; it
     faithfully measures ELF ded4edcb2b6f, which no longer exists. Selecting work
     from it means optimising a binary nobody ships.

So: group by the FULL lane key including dtype, keep the newest certified row per
lane, and flag any surviving loss whose ELF is not the newest ELF that lane has
ever been measured with -- those need a re-measure before they are believed.

"Newest" means the MEASUREMENT time each bench document carries, never the file
mtime (br-frankenpandas-rc0923-epic-zero-certified-losses-bss5q.1). A checkout
gives every file the same mtime, so the census of a fresh clone depended on
directory order (3.14x with 35 losses against 3.93x with 14 on the same commit),
and commit 0cf20bc43, which landed 89 rows late, gave four 2026-08 LOSING rows
2026-09-04 mtimes that outranked later winning reruns.

This script does not measure and does not edit anything.

Usage:
    python3 scripts/current_loss_census.py [--dir artifacts/bench] [--all]
"""

from __future__ import annotations

import argparse
import datetime
import glob
import json
import os
import sys

# Where a bench document records when it was measured: the harness writes
# `timestamp` (ISO 8601); a few hand-assembled documents use the others.
TIMESTAMP_KEYS = ("timestamp", "timestamp_utc", "created_at", "date")


def measured_at(document: dict, path: str) -> tuple[float, bool]:
    """When `document` was measured, as POSIX seconds, and whether the document
    itself said so. Only a document with no parseable stamp falls back to its
    file mtime; callers report those, because that order is not reproducible."""
    for key in TIMESTAMP_KEYS:
        value = document.get(key)
        if not isinstance(value, str):
            continue
        try:
            stamp = datetime.datetime.fromisoformat(value.replace("Z", "+00:00"))
        except ValueError:
            continue
        if stamp.tzinfo is None:
            stamp = stamp.replace(tzinfo=datetime.timezone.utc)
        return stamp.timestamp(), True
    return os.path.getmtime(path), False


def measured_date(seconds: float) -> str:
    return datetime.datetime.fromtimestamp(seconds, datetime.timezone.utc).date().isoformat()


def certified(row: dict) -> bool:
    gate = row.get("median_ci_gate") or {}
    clauses = gate.get("clauses") or {}
    return bool(clauses) and bool(gate.get("decidable")) and all(clauses.values())


def elf_of(row: dict) -> str:
    try:
        return row["frankenpandas"]["executable"]["sha256"][:12]
    except (KeyError, TypeError):
        return "?"


def peak_load(row: dict):
    try:
        return float(row["balanced_square"]["host_state"]["loadavg_1min"]["max"])
    except (KeyError, TypeError, ValueError):
        return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dir", default="artifacts/bench")
    parser.add_argument("--all", action="store_true",
                        help="list every lane, not just the losses")
    args = parser.parse_args()

    newest_certified: dict[tuple, tuple] = {}
    newest_elf_seen: dict[tuple, tuple] = {}
    unstamped: list[str] = []

    for path in sorted(glob.glob(os.path.join(args.dir, "*.json"))):
        try:
            with open(path, encoding="utf-8") as handle:
                document = json.load(handle)
        except (OSError, json.JSONDecodeError):
            continue
        if not isinstance(document, dict):
            continue
        when, stamped = measured_at(document, path)
        if not stamped and document.get("results"):
            unstamped.append(os.path.basename(path))
        # Ties (one invocation writing several files) break on the file name,
        # so the census never depends on directory order.
        order = (when, os.path.basename(path))
        for row in document.get("results") or []:
            if not isinstance(row, dict):
                continue
            key = (row.get("category"), row.get("workload"),
                   row.get("size"), row.get("dtype"))
            # Track the newest ELF this lane was EVER run with, certified or not:
            # an uncertified run on a newer build still proves the old build is
            # superseded.
            if key not in newest_elf_seen or order > newest_elf_seen[key][0]:
                newest_elf_seen[key] = (order, elf_of(row))
            if not certified(row):
                continue
            try:
                ratio = float(row["ratio"])
            except (KeyError, TypeError, ValueError):
                continue
            if key not in newest_certified or order > newest_certified[key][0]:
                newest_certified[key] = (order, ratio, row.get("verdict"),
                                         elf_of(row), peak_load(row))

    if unstamped:
        print(f"warning: {len(unstamped)} bench document(s) carry no timestamp and "
              f"were ordered by file mtime: {', '.join(unstamped)}", file=sys.stderr)

    lanes = sorted(newest_certified.items(), key=lambda kv: kv[1][1])
    losses = [(k, v) for k, v in lanes if v[1] < 1.0]
    shown = lanes if args.all else losses

    print(f"lanes with a certified row : {len(newest_certified)}")
    print(f"  still losing (ratio < 1) : {len(losses)}")
    print()
    header = (f"{'ratio':>8s}  {'workload':34s} {'size':>5s} {'dtype':8s} "
              f"{'ELF':13s} {'load':>6s}  {'measured':10s} note")
    print(header)
    print("-" * len(header))

    suspect = 0
    for key, ((when, _), ratio, verdict, elf, load) in shown:
        category, workload, size, dtype = key
        newest_elf = newest_elf_seen.get(key, (0, elf))[1]
        note = ""
        if newest_elf != elf:
            note = f"STALE ELF — lane later run on {newest_elf}; RE-MEASURE"
            suspect += 1
        elif load is not None and load > 40:
            note = f"peak load {load:.0f} — load-contaminated, re-measure"
        print(f"{ratio:8.3f}  {str(workload)[:34]:34s} {str(size):>5s} {str(dtype):8s} "
              f"{elf:13s} {('%.0f' % load) if load is not None else '?':>6s}  "
              f"{measured_date(when):10s} {note}")

    if suspect:
        print(f"\n{suspect} of the rows above were measured on an ELF the lane has "
              f"since been run on again.\nDo NOT select work from those without "
              f"re-measuring: a certified row is a true statement\nabout the binary "
              f"it measured, which is not necessarily the binary you would change.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
