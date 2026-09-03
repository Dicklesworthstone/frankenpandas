#!/usr/bin/env python3
"""Generate artifacts/perf/SCORECARD.md from the CERTIFIED vs-pandas census.

Rewritten 2026-09-02 (reality check). The previous generator read a single
"latest.json" that did not exist, ignored every row's verdict and A/A-null
gate (so NULL_UNDECIDABLE and DROPPED_HIGH_CV rows were folded into the
geomeans), knew 6 of the harness's 11 categories, and scored an EMPTY category
as 1.0x. SCORECARD.md itself was hand-written and its verdict table dated from
2026-06-02, while README pointed readers at it as "the current scorecard".

This version is the scorecard's only source:

  * scans artifacts/bench/*.json (schema v3/v4 rows under "results");
  * keeps, per lane (category, workload, size, dtype), the NEWEST row whose
    median-CI gate is decidable with every clause true — exactly the rule
    scripts/current_loss_census.py applies — and records every other lane's
    latest verdict separately so undecidable/dropped rows are COUNTED, never
    averaged;
  * reports every category present in the corpus, with an explicit
    "no certified lane" line instead of an invented 1.0x;
  * flags a certified loss whose ELF is not the newest ELF that lane has since
    been run on (a certificate attests to a binary, not to HEAD).

Usage:
    python3 scripts/gen_perf_scorecard.py                    # prints markdown
    python3 scripts/gen_perf_scorecard.py --write            # rewrites SCORECARD.md
    python3 scripts/gen_perf_scorecard.py --json out.json    # machine-readable

Read-only unless --write/--json is given. It never runs a benchmark.
"""
from __future__ import annotations

import argparse
import datetime as dt
import glob
import json
import math
import os
import subprocess
import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent
BENCH_DIR = PROJECT_ROOT / "artifacts" / "bench"
SCORECARD = PROJECT_ROOT / "artifacts" / "perf" / "SCORECARD.md"
HISTORY_MARKER = "## Historical narrative (pre-2026-09, hand-maintained)"


def certified(row: dict) -> bool:
    gate = row.get("median_ci_gate") or {}
    clauses = gate.get("clauses") or {}
    return bool(clauses) and bool(gate.get("decidable")) and all(clauses.values())


def elf_of(row: dict) -> str:
    try:
        return row["frankenpandas"]["executable"]["sha256"][:12]
    except (KeyError, TypeError):
        return "?"


def fp_threads(row: dict):
    try:
        return int(row["frankenpandas"]["thread_count_actually_used"])
    except (KeyError, TypeError, ValueError):
        return None


def pd_threads(row: dict):
    try:
        return int(row["pandas"]["thread_count_actually_used"])
    except (KeyError, TypeError, ValueError):
        return None


def geomean(values: list[float]) -> float | None:
    positive = [v for v in values if v > 0]
    if not positive:
        return None
    return math.exp(sum(math.log(v) for v in positive) / len(positive))


def git_head() -> str:
    try:
        return subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"], cwd=PROJECT_ROOT,
            capture_output=True, text=True, check=True,
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def scan(bench_dir: Path) -> dict:
    newest_certified: dict[tuple, dict] = {}
    newest_any: dict[tuple, dict] = {}
    newest_elf_seen: dict[tuple, tuple] = {}
    files = 0
    rows = 0
    for path in sorted(glob.glob(os.path.join(bench_dir, "*.json"))):
        try:
            with open(path, encoding="utf-8") as handle:
                document = json.load(handle)
        except (OSError, json.JSONDecodeError):
            continue
        results = document.get("results")
        if not isinstance(results, list):
            continue
        files += 1
        mtime = os.path.getmtime(path)
        for row in results:
            if not isinstance(row, dict):
                continue
            rows += 1
            key = (row.get("category"), row.get("workload"), row.get("size"), row.get("dtype"))
            if key[0] is None or key[1] is None:
                continue
            record = {
                "mtime": mtime,
                "file": os.path.basename(path),
                "verdict": row.get("verdict"),
                "elf": elf_of(row),
                "fp_threads": fp_threads(row),
                "pd_threads": pd_threads(row),
            }
            try:
                record["ratio"] = float(row["ratio"])
            except (KeyError, TypeError, ValueError):
                record["ratio"] = None
            if key not in newest_elf_seen or mtime > newest_elf_seen[key][0]:
                newest_elf_seen[key] = (mtime, record["elf"])
            if key not in newest_any or mtime > newest_any[key]["mtime"]:
                newest_any[key] = record
            if certified(row) and record["ratio"] is not None:
                if key not in newest_certified or mtime > newest_certified[key]["mtime"]:
                    newest_certified[key] = record
    for key, record in newest_certified.items():
        record["stale_elf"] = newest_elf_seen[key][1] != record["elf"]
    return {
        "files": files,
        "rows": rows,
        "certified": newest_certified,
        "latest": newest_any,
    }


def summarize(scan_result: dict) -> dict:
    certified_rows = scan_result["certified"]
    latest = scan_result["latest"]
    categories: dict[str, dict] = {}
    for key in latest:
        categories.setdefault(key[0], {"lanes": 0, "certified": 0, "ratios": [],
                                       "losses": 0, "verdicts": {}})
    for key, record in latest.items():
        cat = categories[key[0]]
        cat["lanes"] += 1
        verdict = record["verdict"] or "(none)"
        cat["verdicts"][verdict] = cat["verdicts"].get(verdict, 0) + 1
    for key, record in certified_rows.items():
        cat = categories[key[0]]
        cat["certified"] += 1
        cat["ratios"].append(record["ratio"])
        if record["ratio"] < 1.0:
            cat["losses"] += 1
    for cat in categories.values():
        cat["geomean"] = geomean(cat["ratios"])
        cat["min"] = min(cat["ratios"]) if cat["ratios"] else None
        cat["max"] = max(cat["ratios"]) if cat["ratios"] else None
    all_ratios = [r["ratio"] for r in certified_rows.values()]
    losses = sorted(
        ((key, rec) for key, rec in certified_rows.items() if rec["ratio"] < 1.0),
        key=lambda kv: kv[1]["ratio"],
    )
    newest = max((r["mtime"] for r in latest.values()), default=None)
    oldest_cert = min((r["mtime"] for r in certified_rows.values()), default=None)
    return {
        "categories": dict(sorted(categories.items())),
        "overall_geomean": geomean(all_ratios),
        "certified_lanes": len(certified_rows),
        "total_lanes": len(latest),
        "losses": losses,
        "newest_row": newest,
        "oldest_certified": oldest_cert,
        "files": scan_result["files"],
        "rows": scan_result["rows"],
    }


def fmt_ratio(value) -> str:
    return "—" if value is None else f"{value:.3f}x"


def fmt_date(ts) -> str:
    return "—" if ts is None else dt.date.fromtimestamp(ts).isoformat()


def render_markdown(summary: dict, head: str) -> str:
    today = dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%MZ")
    lines = [
        "# vs-pandas Scorecard (certified census)",
        "",
        f"Generated: {today} · main @ {head} · `python3 scripts/gen_perf_scorecard.py --write`",
        "",
        "Every ratio here is **pandas time ÷ FrankenPandas time** from a row whose "
        "incumbent ran LIVE in the same invocation, with an A/A null control per arm "
        "and a bootstrap median-CI gate whose three clauses all passed. Rows that failed "
        "the gate (NULL_UNDECIDABLE, DROPPED_HIGH_CV, CONTRACT_INVALID) are **counted, "
        "not averaged**. One row per lane (category, workload, size, dtype): the newest "
        "certified one.",
        "",
        f"- Bench corpus: {summary['files']} files, {summary['rows']} rows, "
        f"{summary['total_lanes']} lanes; newest row {fmt_date(summary['newest_row'])}.",
        f"- Certified lanes: **{summary['certified_lanes']}** "
        f"(oldest certified row still standing: {fmt_date(summary['oldest_certified'])}).",
        f"- Overall certified geomean: **{fmt_ratio(summary['overall_geomean'])}**.",
        f"- Certified lanes still losing: **{len(summary['losses'])}**.",
        "",
        "## Per category",
        "",
        "| Category | Lanes | Certified | Geomean | Min | Max | Losing | Latest verdicts (all lanes) |",
        "|---|---:|---:|---:|---:|---:|---:|---|",
    ]
    for name, cat in summary["categories"].items():
        verdicts = ", ".join(f"{k} {v}" for k, v in sorted(cat["verdicts"].items(), key=lambda kv: -kv[1]))
        geo = fmt_ratio(cat["geomean"]) if cat["certified"] else "no certified lane"
        lines.append(
            f"| {name} | {cat['lanes']} | {cat['certified']} | {geo} | "
            f"{fmt_ratio(cat['min'])} | {fmt_ratio(cat['max'])} | {cat['losses']} | {verdicts} |"
        )
    lines += [
        "",
        "A category with no certified lane is reported as such; it is not parity.",
        "",
        "## Certified losses (newest certified row per lane)",
        "",
    ]
    if not summary["losses"]:
        lines.append("None.")
    else:
        lines += [
            "| Ratio | Category | Workload | Size | dtype | FP thr | pd thr | ELF | Measured | Note |",
            "|---:|---|---|---|---|---:|---:|---|---|---|",
        ]
        for key, rec in summary["losses"]:
            note = "STALE ELF: lane since run on a newer binary; re-measure before acting" if rec["stale_elf"] else ""
            lines.append(
                f"| {rec['ratio']:.3f}x | {key[0]} | `{key[1]}` | {key[2]} | {key[3]} | "
                f"{rec['fp_threads'] if rec['fp_threads'] is not None else '?'} | "
                f"{rec['pd_threads'] if rec['pd_threads'] is not None else '?'} | "
                f"`{rec['elf']}` | {fmt_date(rec['mtime'])} | {note} |"
            )
    lines += [
        "",
        "Thread columns are what each arm ACTUALLY used (harness-recorded). A win with "
        "FP on 8 threads against pandas on 1 is a real engine comparison, but not a "
        "single-threaded one; the harness's like_for_like block flags thread capping.",
        "",
        "## How to read a loss",
        "",
        "1. Re-measure it on a binary built today (`scripts/current_loss_census.py` flags stale ELFs).",
        "2. A loss that reproduces across weeks is structural; the three known floors are "
        "2-D block storage (transpose / `.values`), str-key groupby factorization, and the "
        "df_dot GEMM microkernel (`docs/repo_vs_pandas_assessment_dustysummit.md`).",
        "3. Per AGENTS.md, reporting a loss is a success; a self-speedup is maintenance, not a win.",
        "",
    ]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--dir", type=Path, default=BENCH_DIR)
    parser.add_argument("--write", action="store_true", help="rewrite artifacts/perf/SCORECARD.md")
    parser.add_argument("--json", type=Path, help="also write the summary as JSON here")
    args = parser.parse_args()

    if not args.dir.is_dir():
        print(f"error: {args.dir} is not a directory", file=sys.stderr)
        return 1
    summary = summarize(scan(args.dir))
    markdown = render_markdown(summary, git_head())

    if args.json:
        payload = {
            "generated": dt.datetime.now(dt.timezone.utc).isoformat(),
            "certified_lanes": summary["certified_lanes"],
            "total_lanes": summary["total_lanes"],
            "overall_geomean": summary["overall_geomean"],
            "categories": {
                name: {k: v for k, v in cat.items() if k != "ratios"}
                for name, cat in summary["categories"].items()
            },
            "losses": [
                {"category": k[0], "workload": k[1], "size": k[2], "dtype": k[3], **{kk: vv for kk, vv in rec.items() if kk != "mtime"}, "measured": fmt_date(rec["mtime"])}
                for k, rec in summary["losses"]
            ],
        }
        args.json.write_text(json.dumps(payload, indent=2) + "\n")

    if args.write:
        history = ""
        if SCORECARD.exists():
            existing = SCORECARD.read_text()
            if HISTORY_MARKER in existing:
                history = existing[existing.index(HISTORY_MARKER):]
            else:
                # First regeneration: keep the old hand-written file as history.
                history = (
                    f"{HISTORY_MARKER}\n\n"
                    "Everything below this line predates the generated census above, was "
                    "maintained by hand, and its verdict table (dated 2026-06-02) is NOT the "
                    "current state. It is kept for the lever-by-lever narrative only.\n\n"
                    + existing
                )
        SCORECARD.write_text(markdown + "\n" + history)
        print(f"wrote {SCORECARD}")
    else:
        print(markdown)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
