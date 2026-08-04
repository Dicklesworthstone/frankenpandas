#!/usr/bin/env python3
"""KEEP-claim incumbent coverage at HEAD — final counter.

Fixes the two parse bugs found in earlier passes:
  1. verdict cells are emoji/bold decorated ('✅ KEEP — ...'), so a bare
     startswith('KEEP') drops the majority;
  2. long ledger tables are interrupted by blank lines, so a naive
     "first row of a | run is the header" rule treats continuation blocks'
     DATA rows as headers. Continuations now inherit the last real header
     with the same column count.

Classification of each KEEP row:
  SAME_INVOCATION_VS_PANDAS : pandas measured live in the same process
  FP_SIDE_NULL_GATED        : corrected A/A null gate, but comparator is FP
  CROSS_PROCESS_VS_PANDAS   : has a pandas number, taken separately
  NO_INCUMBENT              : no pandas number at all
"""
from __future__ import annotations

import json
import re
from collections import Counter
from pathlib import Path

ROOT = Path("/data/projects/frankenpandas")
LEDGER = ROOT / "docs" / "NEGATIVE_EVIDENCE.md"
BENCH_DIR = ROOT / "artifacts" / "bench"
VERDICTS = {"KEEP", "REJECT", "VOID", "WEAK", "DROPPED", "FIXED", "PARITY",
            "SLOWER", "FASTER", "PASS", "FAIL", "INCOMPLETE", "SUPERSEDED"}


def tok(cell: str) -> str:
    s = re.sub(r"^[^0-9A-Za-z]+", "", cell).lstrip("*_ ")
    m = re.match(r"[A-Za-z_]+", s)
    return m.group(0).upper() if m else ""


def cells(line: str) -> list[str]:
    return [x.strip() for x in line.strip().strip("|").split("|")]


def is_sep(cs: list[str]) -> bool:
    return bool(cs) and all(re.fullmatch(r":?-{2,}:?", c) for c in cs)


def looks_like_header(cs: list[str]) -> bool:
    """A header row has no verdict token in its last cell and no ms/x numbers."""
    if not cs:
        return False
    if tok(cs[-1]) in VERDICTS and tok(cs[-1]) != "":
        return False
    joined = " ".join(cs).lower()
    if re.search(r"\d+(\.\d+)?\s*(ms|us|µs|s\b|×|x faster)", joined):
        return False
    return True


def parse_rows(lines: list[str]):
    """Yield (lineno, header, row_cells, raw) with continuation-aware headers."""
    headers_by_width: dict[int, list[str]] = {}
    i, n = 0, len(lines)
    while i < n:
        if not lines[i].startswith("|"):
            i += 1
            continue
        start = i
        while i < n and lines[i].startswith("|"):
            i += 1
        block = lines[start:i]
        block_header: list[str] | None = None
        for off, raw in enumerate(block):
            cs = cells(raw)
            if is_sep(cs):
                continue
            if off == 0 and looks_like_header(cs):
                block_header = cs
                headers_by_width[len(cs)] = cs
                continue
            hdr = block_header
            if hdr is None or len(hdr) != len(cs):
                hdr = headers_by_width.get(len(cs))
            yield (start + off + 1, hdr, cs, raw)


def is_commit_replay(hdr: list[str] | None) -> bool:
    if not hdr:
        return False
    low = [h.lower() for h in hdr]
    return low[0] in ("#",) and any("commit" in h for h in low)


SAME_INV_ROW = ("vs_pandas_harness", "null_control", "median_ci_gate")


def main() -> None:
    lines = LEDGER.read_text(errors="replace").splitlines()
    buckets = Counter()
    rows_by_bucket: dict[str, list] = {}
    excluded = Counter()

    for lineno, hdr, cs, raw in parse_rows(lines):
        if not cs or tok(cs[-1]) != "KEEP":
            continue
        if is_commit_replay(hdr):
            excluded["commit-replay (not a perf claim)"] += 1
            continue

        hdr_low = " ".join(h.lower() for h in (hdr or []))
        blob = raw.lower()
        has_aa_null = ("a/a" in hdr_low) or ("null" in hdr_low)
        # does an incumbent (pandas) number appear?
        pd_idx = next((i for i, h in enumerate(hdr or []) if "pandas" in h.lower()),
                      None)
        pd_cell = cs[pd_idx] if (pd_idx is not None and pd_idx < len(cs)) else ""
        mentions_pandas = "pandas" in blob or "vs pandas" in blob
        has_pd_number = bool(re.search(r"\d", pd_cell)) or (
            mentions_pandas and bool(re.search(r"\d+(\.\d+)?\s*[x×]", raw))
        )

        if any(m in blob for m in SAME_INV_ROW) and mentions_pandas:
            b = "SAME_INVOCATION_VS_PANDAS"
        elif has_aa_null:
            b = "FP_SIDE_NULL_GATED"
        elif has_pd_number:
            b = "CROSS_PROCESS_VS_PANDAS"
        else:
            b = "NO_INCUMBENT"
        buckets[b] += 1
        rows_by_bucket.setdefault(b, []).append((lineno, cs[0][:78], pd_cell[:26]))

    total = sum(buckets.values())
    same = buckets["SAME_INVOCATION_VS_PANDAS"]

    print("=" * 76)
    print("KEEP-CLAIM INCUMBENT COVERAGE — FrankenPandas @ HEAD")
    print("=" * 76)
    print(f"A. total KEEP perf claims held             : {total}")
    print(f"B. incumbent LIVE in the same invocation   : {same}")
    print(f"C. NOT measured that way                   : {total - same}")
    print()
    print("Breakdown of C:")
    for b in ("FP_SIDE_NULL_GATED", "CROSS_PROCESS_VS_PANDAS", "NO_INCUMBENT"):
        print(f"   {b:28s} {buckets[b]}")
    print()
    for k, v in excluded.most_common():
        print(f"excluded: {k}: {v}")
    print()
    for b in ("NO_INCUMBENT", "FP_SIDE_NULL_GATED"):
        rs = rows_by_bucket.get(b, [])
        if not rs:
            continue
        print(f"--- {b} ({len(rs)}) ---")
        for ln, lever, pdc in rs[:30]:
            print(f"   L{ln}: {lever}")
        print()

    census = Counter()
    null_rows = files = 0
    for path in sorted(BENCH_DIR.glob("*.json")):
        try:
            data = json.loads(path.read_text())
        except Exception:
            continue
        files += 1
        for c in (data.get("comparisons") or data.get("results") or []):
            if not isinstance(c, dict):
                continue
            if c.get("verdict"):
                census[c["verdict"]] += 1
            fp = c.get("frankenpandas")
            if isinstance(fp, dict) and fp.get("null_control"):
                null_rows += 1
    print("-" * 76)
    print(f"HARNESS ARTIFACTS: {files} files, {sum(census.values())} comparison rows")
    print(f"  rows carrying a null_control block: {null_rows}")
    for v, n in census.most_common():
        print(f"   {v:24s} {n}")


if __name__ == "__main__":
    main()
