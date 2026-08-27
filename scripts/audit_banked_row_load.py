#!/usr/bin/env python3
"""Audit banked vs-pandas rows for LOAD CONTAMINATION, and give the bias a sign.

br-frankenpandas-633fb. `loadavg_1min` is recorded on every balanced-square row
and gated on nothing, so a row can pass all three median-CI clauses AND both A/A
null controls while the host is saturated. One did: `df_dot @100k` certified
FASTER 16.63x at loadavg 87.8-102.4, while the same ELF measured 6.323x at
loadavg 45.3-92.1 -- a 2.6x spread on byte-identical code.

The A/A null cannot see this by construction: both arms of the null are starved
EQUALLY, so their ratio stays near unity while the subject-versus-incumbent ratio
skews.

THE BIAS HAS A SIGN, and `thread_provenance.thread_count_actually_used` already
records what determines it. Competing load starves whichever arm asks for more
threads, so:

    pandas threads > FrankenPandas threads  ->  our ratio is INFLATED
                                                a WIN here is suspect
    FrankenPandas threads > pandas threads  ->  our ratio is DEFLATED
                                                a WIN here is conservative, a
                                                LOSS is suspect
    equal                                    ->  unsigned; still noisy

This script does not re-measure and does not edit anything. It reads the banked
corpus and prints which rows a load gate would have refused, so nobody has to
re-derive the list by hand.

Usage:
    python3 scripts/audit_banked_row_load.py [--load-max N] [--dir artifacts/bench]
"""

from __future__ import annotations

import argparse
import glob
import json
import os
import statistics


def iter_rows(directory: str):
    for path in sorted(glob.glob(os.path.join(directory, "*.json"))):
        try:
            with open(path, encoding="utf-8") as handle:
                document = json.load(handle)
        except (OSError, json.JSONDecodeError):
            continue
        results = document.get("results")
        if not isinstance(results, list):
            continue
        for row in results:
            if isinstance(row, dict):
                yield path, row


def certified(row: dict) -> bool:
    gate = row.get("median_ci_gate") or {}
    clauses = gate.get("clauses") or {}
    return bool(clauses) and bool(gate.get("decidable")) and all(clauses.values())


def load_window(row: dict):
    try:
        window = row["balanced_square"]["host_state"]["loadavg_1min"]
        return float(window["min"]), float(window["max"])
    except (KeyError, TypeError, ValueError):
        return None


def threads(row: dict):
    try:
        used = row["thread_provenance"]["thread_count_actually_used"]
        return used.get("frankenpandas"), used.get("pandas")
    except (KeyError, TypeError):
        return None, None


def bias(fp_threads, pandas_threads) -> str:
    if fp_threads is None or pandas_threads is None:
        return "unknown"
    if pandas_threads > fp_threads:
        return "INFLATED (win suspect)"
    if fp_threads > pandas_threads:
        return "deflated (win conservative)"
    return "unsigned (equal threads)"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--load-max", type=float, default=40.0,
                        help="flag certified rows whose peak 1-minute loadavg exceeds this")
    parser.add_argument("--dir", default="artifacts/bench")
    args = parser.parse_args()

    total = flagged = missing_load = 0
    peaks: list[float] = []
    rows = []
    for path, row in iter_rows(args.dir):
        if not certified(row):
            continue
        total += 1
        window = load_window(row)
        if window is None:
            missing_load += 1
            continue
        low, high = window
        peaks.append(high)
        if high > args.load_max:
            fp_threads, pandas_threads = threads(row)
            rows.append((high, low, fp_threads, pandas_threads, row, os.path.basename(path)))
            flagged += 1

    print(f"certified rows (decidable, all clauses TRUE): {total}")
    print(f"  loadavg recorded: {total - missing_load}    NOT recorded: {missing_load}")
    if peaks:
        ordered = sorted(peaks)
        print(f"  peak loadavg  min={ordered[0]:.1f}  p50={statistics.median(ordered):.1f}  "
              f"p90={ordered[int(0.9 * len(ordered))]:.1f}  max={ordered[-1]:.1f}")
    print(f"\nrows a --load-max {args.load_max:g} gate would REFUSE: {flagged}\n")

    header = f"{'load window':>16}  {'fp_thr':>6} {'pd_thr':>6}  {'bias':28s} row"
    print(header)
    print("-" * len(header))
    for high, low, fp_threads, pandas_threads, row, name in sorted(rows, reverse=True):
        print(f"{low:7.1f}-{high:7.1f}  {str(fp_threads):>6} {str(pandas_threads):>6}  "
              f"{bias(fp_threads, pandas_threads):28s} "
              f"{row.get('workload')} @{row.get('size')} "
              f"{row.get('verdict')} {row.get('ratio')}   [{name}]")

    suspect = [r for r in rows if r[2] is not None and r[3] is not None and r[3] > r[2]
               and str(r[4].get("verdict")) == "FASTER"]
    if suspect:
        print(f"\nWINS whose bias runs IN OUR FAVOUR (re-measure before quoting): {len(suspect)}")
        for high, low, fp_threads, pandas_threads, row, _ in sorted(suspect, reverse=True):
            print(f"  {row.get('workload')} @{row.get('size')}  {row.get('ratio')}x  "
                  f"load {low:.1f}-{high:.1f}  threads fp={fp_threads} pandas={pandas_threads}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
