#!/usr/bin/env python3
"""Is the host quiet RIGHT NOW? `uptime` cannot answer that; this can.

WHY THIS EXISTS (the four questions section 2 of the standing orders requires
before an artifact may be created):

  * OBSERVED DEFECT CLASS. 2026-08-17: a certification window read loadavg 21.46
    -- comfortably under every threshold this campaign uses -- while `ps` showed
    4343% of CPU in `rustc`, i.e. 43 cores compiling, with elapsed times of 0-4
    seconds. Twenty seconds later loadavg had climbed to 40.42 chasing a storm
    that was already at full intensity when the decision was made. Loadavg is an
    exponentially-damped average with a ~60s time constant, so a build storm that
    began seconds ago is INVISIBLE to it at exactly the moment an agent decides
    whether to measure. This repeatedly produced rows that "entered at an
    acceptable load and ended at three times that", whose A/A nulls then failed.
  * CONCRETE CONSUMER. Any agent about to spend two to five minutes on a
    certification run, on a host shared by twenty panes.
  * THE GATE IT ENFORCES. None. It prints a verdict and exits 0/1 so a human or
    agent can decide; it blocks nothing and no measurement branches on it. It is
    a look-before-you-leap instrument, not a policy.
  * DELETION CONDITION. Delete it when the benchmark harness refuses to start
    inside a storm on its own -- it already samples per-CPU busy fractions for
    `host-wide-exclusive` mode, so the capability is close by. At that point this
    is redundant.

Read-only: parses /proc, starts nothing, writes nothing, and takes well under a
second. Costs no compute worth measuring, which is the whole point -- the check
has to be cheap enough to run before EVERY row.
"""

from __future__ import annotations

import argparse
import pathlib
import sys

# Compiler and build-tool process names worth counting. `cargo` itself is mostly
# idle while it waits on children, but it is cheap to include and its presence is
# informative when rustc has not yet been spawned.
BUILD_COMMS = ("rustc", "cargo", "cc1plus", "cc1", "ld", "lld", "clang", "gcc", "make", "ninja")

# One core's worth of compilation is noise; ten cores is a storm. The default sits
# between them, deliberately closer to the low end: the cost of deferring a row is
# a few minutes, and the cost of measuring inside a storm is a wasted run PLUS a
# misleading number if it happens to pass.
DEFAULT_BUSY_CPU_PERCENT = 200.0


def read_proc_stat_cpu() -> dict[int, tuple[str, float]]:
    """`{pid: (comm, cpu_percent_since_boot)}` for live processes.

    Uses the same utime+stime/elapsed definition `ps pcpu` reports, which is an
    average over the process's whole life rather than an instantaneous rate. For a
    build that is the right reading: a rustc that has been pegged for four seconds
    reports ~400%, and one that finished its burst reports less.
    """
    clock_ticks = 100.0  # _SC_CLK_TCK on every Linux this campaign runs on
    try:
        with open("/proc/uptime", encoding="utf-8") as handle:
            uptime_seconds = float(handle.read().split()[0])
    except (OSError, ValueError):
        return {}

    out: dict[int, tuple[str, float]] = {}
    for entry in pathlib.Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            stat = (entry / "stat").read_text(encoding="utf-8")
        except (OSError, ProcessLookupError):
            continue  # the process exited between listing and reading; normal
        # comm is parenthesised and may itself contain spaces or ')'.
        open_paren = stat.find("(")
        close_paren = stat.rfind(")")
        if open_paren < 0 or close_paren < 0:
            continue
        comm = stat[open_paren + 1 : close_paren]
        fields = stat[close_paren + 2 :].split()
        try:
            utime, stime, starttime = float(fields[11]), float(fields[12]), float(fields[19])
        except (IndexError, ValueError):
            continue
        elapsed = uptime_seconds - (starttime / clock_ticks)
        if elapsed <= 0:
            continue
        out[int(entry.name)] = (comm, 100.0 * ((utime + stime) / clock_ticks) / elapsed)
    return out


def build_cpu_percent() -> tuple[float, list[tuple[str, float]]]:
    """Total CPU% attributable to build tooling, and the busiest contributors."""
    procs = read_proc_stat_cpu()
    hits = [(comm, pct) for comm, pct in procs.values() if comm in BUILD_COMMS]
    hits.sort(key=lambda item: -item[1])
    return sum(pct for _, pct in hits), hits[:5]


def loadavg_1min() -> float:
    try:
        with open("/proc/loadavg", encoding="utf-8") as handle:
            return float(handle.read().split()[0])
    except (OSError, ValueError):
        return float("nan")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--max-build-cpu",
        type=float,
        default=DEFAULT_BUSY_CPU_PERCENT,
        help="build CPU%% at or above which the host is called busy (default: 200)",
    )
    args = parser.parse_args()

    total, top = build_cpu_percent()
    load = loadavg_1min()
    busy = total >= args.max_build_cpu

    print(f"loadavg 1-min      : {load:.2f}")
    print(f"build CPU right now: {total:.0f}%  (threshold {args.max_build_cpu:.0f}%)")
    for comm, pct in top:
        print(f"    {comm:>8}  {pct:.0f}%")
    if busy and load < 30.0:
        print(
            "\nVERDICT: BUSY, and loadavg does NOT show it yet.\n"
            "This is the case the check exists for: loadavg is damped over ~60s, so a\n"
            "storm that began seconds ago is invisible to it. Defer the run."
        )
    elif busy:
        print("\nVERDICT: BUSY. Defer the run.")
    else:
        print("\nVERDICT: quiet by both readings. A row started now is not entering a storm.")
    return 1 if busy else 0


if __name__ == "__main__":
    raise SystemExit(main())
