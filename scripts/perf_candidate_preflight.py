#!/usr/bin/env python3
"""Perf-lever preflight — makes an unfalsifiable REJECT impossible, not merely discouraged.

Campaign `perf-campaign-20260725`, Meta-Lever #1 institutionalization. Modelled on
frankensqlite's `sql_pipeline_candidate_preflight` (exit 2 = BLOCKED), which is why that
repo sits at a 1.7% void rate after four months while repos that audited once and stopped
sit at 25-91%. Ledger integrity DECAYS; this is the ratchet.

Two modes.

1. `--candidate "<keywords>"` — grep the ledger BEFORE you write a lever. If a prior row
   already rejected this surface, you are BLOCKED (exit 2) with the row cited. Agents in
   this fleet have re-derived already-rejected levers inside a single turn.

2. `--check-new-rejects` (default; also the pre-commit mode) — every NEW `###` REJECT
   section added to `docs/NEGATIVE_EVIDENCE.md` must carry at least one of:
     * an A/A null control (the effect was decided against a measured floor), OR
     * a counted mechanism shown UNCHANGED -- instructions / cycles / syscalls /
       allocations / page-branch-cache misses / `perf stat` (a null cannot change the fact
       that no work was removed), OR
     * an explicit zero-self-time profile attribution (the bench never ran the candidate).
   A row with none of these cannot distinguish the lever from the harness. That is
   VOID-NONULL, the class that is 90.2% of this repo's void rows and 97.7% of frankenfs's.

   NOTE ON PARITY vs MECHANISM: "bit-identical" / "byte-identical" / "0 diffs" are PARITY
   proofs (the output is unchanged). They are NOT mechanism refutations (the work is
   unchanged) and are deliberately NOT accepted here. Conflating them is exactly how an
   audit silently launders ~100 never-refuted rejects -- it happened in this repo's own
   first screen (102 rows -> 3 after tightening).

Exit codes: 0 = pass, 2 = BLOCKED, 1 = usage/IO error.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
LEDGER = REPO / "docs" / "NEGATIVE_EVIDENCE.md"

REJECT_MARK = re.compile(
    r"\bREJECT(?:ED|S)?\b|\bNOSHIP\b|\bNO-SHIP\b|zero-gain|~0-gain", re.I
)
NULL_CTRL = re.compile(
    r"null control|null-control|A/A|null arm|null floor|nulls?\s*~\s*1\.0|paired\(", re.I
)
# Requires a COUNTED quantity shown unchanged. Deliberately excludes bit/byte-identical.
MECHANISM = re.compile(
    r"(?:instructions?|cycles|syscalls?|allocations?|mallocs?|page[- ]faults?|"
    r"branch[- ]miss(?:es)?|cache[- ]miss(?:es)?|IPC)\s*"
    r"(?:count\w*\s*)?(?:are|is|was|were|remained?|stayed?|:)?\s*"
    r"(?:un|not )?chang\w*|perf stat|identical instruction|same instruction count|"
    r"no work (?:was )?removed",
    re.I,
)
ZERO_SELF = re.compile(
    r"0\.000+\s*%|0\.000000s|zero self-?time|no self-?time|never (?:ran|executed|routed)",
    re.I,
)


def _git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=REPO, capture_output=True, text=True, check=False
    ).stdout


def added_ledger_sections(base: str) -> list[tuple[str, str]]:
    """Return (title, body) for each `###` section added to the ledger vs `base`."""
    diff = _git("diff", "--unified=0", base, "--", str(LEDGER.relative_to(REPO)))
    added = [l[1:] for l in diff.splitlines() if l.startswith("+") and not l.startswith("+++")]
    sections: list[tuple[str, list[str]]] = []
    for line in added:
        if line.startswith("### "):
            sections.append((line[4:].strip(), []))
        elif sections:
            sections[-1][1].append(line)
    return [(t, "\n".join(b)) for t, b in sections]


def check_new_rejects(base: str) -> int:
    sections = added_ledger_sections(base)
    if not sections:
        print(f"preflight: no new ledger sections vs {base} — OK")
        return 0

    blocked = []
    for title, body in sections:
        blob = f"{title}\n{body}"
        if not REJECT_MARK.search(blob):
            continue  # KEEP / survey rows are not gated
        if NULL_CTRL.search(blob) or MECHANISM.search(blob) or ZERO_SELF.search(blob):
            continue
        blocked.append(title)

    if blocked:
        print("preflight: BLOCKED — new REJECT row(s) with no falsifiable basis\n")
        for t in blocked:
            print(f"  ✗ {t[:150]}")
        print(
            "\nEvery REJECT must record ONE of:\n"
            "  (a) an A/A null control          -- the effect was decided against a measured floor\n"
            "  (b) a COUNTED mechanism unchanged -- instructions/cycles/syscalls/allocations/faults\n"
            "  (c) ~0% target self-time          -- the bench never executed the candidate\n\n"
            "Without one of these the row cannot distinguish the LEVER from the HARNESS, which is\n"
            "the VOID-NONULL class: 90.2% of this repo's void rows (see docs/LEDGER_RESURRECTION.md).\n"
            "'bit-identical' does NOT satisfy (b) -- that is a parity proof, not a mechanism refutation.\n"
        )
        return 2

    print(f"preflight: {len(sections)} new ledger section(s), all REJECTs falsifiable — OK")
    return 0


def check_candidate(keywords: str) -> int:
    if not LEDGER.is_file():
        print(f"preflight: ledger not found at {LEDGER}", file=sys.stderr)
        return 1
    terms = [t for t in re.split(r"[\s,]+", keywords.strip()) if len(t) > 2]
    if not terms:
        print("preflight: --candidate needs at least one term of 3+ chars", file=sys.stderr)
        return 1

    hits: list[tuple[int, str]] = []
    current = ""
    for i, line in enumerate(LEDGER.read_text(errors="replace").splitlines(), 1):
        if line.startswith("### "):
            current = line[4:].strip()
        if current and REJECT_MARK.search(current):
            if all(re.search(re.escape(t), current, re.I) for t in terms):
                if not hits or hits[-1][1] != current:
                    hits.append((i, current))

    if hits:
        print(f"preflight: BLOCKED — {len(hits)} prior REJECT row(s) match {terms!r}\n")
        for line_no, title in hits[:10]:
            print(f"  docs/NEGATIVE_EVIDENCE.md:{line_no}\n    {title[:150]}\n")
        print(
            "Read these rows and their retry predicates BEFORE writing code. If you believe a\n"
            "predicate is now satisfied, say so explicitly in your new row and cite the line number.\n"
        )
        return 2

    print(f"preflight: no prior REJECT matches {terms!r} — OK to proceed")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--candidate", help="Keywords for the lever you are about to write")
    ap.add_argument("--check-new-rejects", action="store_true", help="Validate newly added REJECT rows")
    ap.add_argument("--base", default="HEAD", help="Git ref to diff the ledger against (default HEAD)")
    args = ap.parse_args()

    if args.candidate:
        return check_candidate(args.candidate)
    return check_new_rejects(args.base)


if __name__ == "__main__":
    raise SystemExit(main())
