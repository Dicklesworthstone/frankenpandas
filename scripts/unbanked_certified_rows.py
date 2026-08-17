#!/usr/bin/env python3
"""List vs-pandas rows that PASSED the gate and were never written into a ledger.

WHY THIS EXISTS (the four questions section 2 of the standing orders requires an
artifact to answer before it may be created):

  * OBSERVED DEFECT CLASS. On 2026-08-17 a sweep of `artifacts/bench/` found 28
    fully-passing certified rows across 13 workloads -- including `loc_labels`
    at 14.817x, `join_inner` at 10.535x and `dt_strftime` at 10.192x, several
    independently replicated -- that appear in NO ledger or report anywhere in
    the repo. They were produced, gated, and dropped. Producing a row costs a
    measurement window on a contended host; writing it down is free.
  * CONCRETE CONSUMER. An agent finishing a measurement turn, or working through
    a build halt with no host access, runs this and gets a worklist.
  * THE GATE IT ENFORCES. None, deliberately. This does not block a commit and
    nothing branches on its output -- it is a worklist generator, and it is the
    honest shape for the problem, because deciding whether a row should be banked
    or rejected requires judgement this script does not have.
  * DELETION CONDITION. Delete this when the harness banks a passing row itself,
    or when `artifacts/bench/` is no longer the place rows accumulate. At that
    point it will report nothing and should go.

WHY THE MATCHING WORKS THIS WAY, which took three attempts:

  1. Matching on the RATIO fails. "14.8" occurs incidentally throughout a
     27,000-line ledger -- timestamps, other figures, CI bounds -- and reports
     false hits on nearly everything.
  2. Matching on the WORKLOAD NAME fails for the same reason in reverse: the
     names appear in prose that discusses the op without banking the row.
  3. Matching on the row's own NON-COLLIDING figures works. A two-decimal
     microsecond p50 and a six-decimal A/A null median are effectively unique.
     The method self-validates: a row scores 0/4 before it is banked and 4/4
     immediately after, which is exactly what was observed when `log @1M`
     (2.036x) was banked and re-checked.

A ROW'S RATIO IS NOT INTERPRETABLE WITHOUT ITS THREAD REQUEST, which this script
learned the hard way on 2026-08-17: two `str_startswith_arrow @1M` rows in this very
worklist certified at 1.275x and 4.824x, both FASTER, both 3/3 clauses, twenty-seven
minutes apart on one host with NO source change between the two commits. The only
difference was `thread_count_requested`: 1 versus 64. pandas is single-threaded here
so its arm moved 0.6%; FrankenPandas' moved 3.74x. Banking those two side by side as
replicates would have manufactured a contradiction out of a documented parameter, so
the table now prints the thread request and flags any capped row with `!`.

Read-only. Runs no build, opens no network, writes nothing.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent

# Every place a row may legitimately have been written down. Kept broad on
# purpose: a row recorded in ANY of these is not lost, whatever the campaign's
# canonical ledger happens to be this month.
PROSE_GLOBS = (
    "docs/*.md",
    "artifacts/optimization/*.md",
    "reports/**/*.md",
    "*.md",
)

BENCH_GLOB = "artifacts/bench/bench_*.json"


def load_prose() -> str:
    """Concatenate every markdown file a row could have been banked in."""
    chunks: list[str] = []
    for pattern in PROSE_GLOBS:
        for path in REPO.glob(pattern):
            try:
                chunks.append(path.read_text(errors="ignore"))
            except OSError:
                continue
    return "\n".join(chunks)


def row_fingerprint(result: dict) -> list[str]:
    """The figures that cannot collide with unrelated text.

    Two-decimal microsecond medians and six-decimal null medians. Deliberately
    NOT the ratio, and NOT the workload name -- see the module docstring for why
    both of those produce false positives.
    """
    fp = result["frankenpandas"]
    pandas_arm = result["pandas"]
    return [
        f"{fp['p50_us']:.2f}",
        f"{pandas_arm['p50_us']:.2f}",
        f"{fp['null_control']['median_ratio']:.6f}",
        f"{pandas_arm['null_control']['median_ratio']:.6f}",
    ]


def fully_passing(result: dict) -> bool:
    """A row whose verdict is FASTER and whose three gate clauses all held.

    Anything NULL_UNDECIDABLE, SLOWER, or passing on fewer than three clauses is
    NOT reported: an ungated number is not a lost win, it is a number that was
    correctly withheld.
    """
    if result.get("verdict") != "FASTER":
        return False
    clauses = result.get("median_ci_gate", {}).get("clauses", {})
    return bool(clauses) and all(clauses.values())


def scan(min_ratio: float) -> list[dict]:
    prose = load_prose()
    unbanked: list[dict] = []
    for path in sorted(REPO.glob(BENCH_GLOB)):
        try:
            document = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError):
            # A truncated artifact is not this script's problem to fix, and
            # skipping it is safe: it can only cause an UNDER-report, never a
            # false claim that something is lost.
            continue
        for result in document.get("results", []):
            if not fully_passing(result):
                continue
            if float(result.get("ratio", 0.0)) < min_ratio:
                continue
            hits = sum(1 for token in row_fingerprint(result) if token in prose)
            if hits:
                continue
            provenance = result.get("thread_provenance", {})
            host_threads = provenance.get("logical_threads")
            # `thread_count_requested` is None on an unconstrained run and
            # `thread_count_actually_used` reads 1 even on a 64-way run, so neither
            # can be trusted alone. `runtime_available_parallelism` is what the
            # process could actually see, and it is the field that separated the
            # 1.275x row from the 4.824x one.
            available = provenance.get("runtime_available_parallelism") or {}
            requested = available.get("frankenpandas") or provenance.get(
                "thread_count_requested"
            )
            unbanked.append(
                {
                    "ratio": result["ratio"],
                    "threads_requested": requested,
                    "host_logical_threads": host_threads,
                    # A row measured under a thread cap is NOT comparable to an
                    # unconstrained one and must never be tabulated beside it --
                    # see the module docstring.
                    "thread_capped": bool(
                        requested and host_threads and requested < host_threads
                    ),
                    "category": result.get("category", "?"),
                    "workload": result.get("workload", "?"),
                    "size": result.get("size", "?"),
                    "fp_p50_us": result["frankenpandas"]["p50_us"],
                    "pandas_p50_us": result["pandas"]["p50_us"],
                    "elf": result["frankenpandas"]["executable"]["sha256"][:16],
                    "invocation_id": document.get("invocation_id", ""),
                    "artifact": str(path.relative_to(REPO)),
                }
            )
    unbanked.sort(key=lambda row: -row["ratio"])
    return unbanked


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--min-ratio",
        type=float,
        default=1.0,
        help="only report rows at or above this ratio (default: 1.0, i.e. all wins)",
    )
    parser.add_argument("--json", action="store_true", help="emit JSON instead of a table")
    args = parser.parse_args()

    rows = scan(args.min_ratio)
    if args.json:
        json.dump(rows, sys.stdout, indent=2)
        sys.stdout.write("\n")
        return 0

    if not rows:
        print("no unbanked fully-passing rows found — every certified row is written down")
        return 0

    print(f"{len(rows)} fully-passing rows are recorded in no ledger or report:\n")
    print(f"{'ratio':>9}  {'thr':>5}  {'category':>12}  {'workload':>24}  {'size':>5}  artifact")
    for row in rows:
        threads = row["threads_requested"]
        marker = f"{threads}!" if row["thread_capped"] else (str(threads) if threads else "?")
        print(
            f"{row['ratio']:>8}x  {marker:>5}  {row['category']:>12}  {row['workload']:>24}  "
            f"{row['size']:>5}  {row['artifact']}"
        )
    capped = [row for row in rows if row["thread_capped"]]
    if capped:
        print(
            f"\n{len(capped)} row(s) marked `!` ran under a THREAD CAP below this host's "
            "core count.\nThey are honest rows and the harness recorded the cap, but their "
            "ratio answers a\ndifferent question and must not be tabulated beside "
            "unconstrained rows. Observed:\n`str_startswith_arrow @1M` certified 3/3 clauses "
            "at 1.275x on one core and 4.824x on\nsixty-four, same host, same day, no source "
            "change between them -- pandas' arm moved\n0.6% and FrankenPandas' moved 3.74x. "
            "The capped row also had the CLEANEST A/A nulls\nof the batch (0.99960/0.99997), "
            "because one busy core is a quiet host: null quality\nis not representativeness."
        )
    print(
        "\nEach needs a decision, not a rerun: read the artifact, then bank it or "
        "record why it was rejected.\nBoth outcomes are campaign output; leaving it "
        "here is the only outcome that is not."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
