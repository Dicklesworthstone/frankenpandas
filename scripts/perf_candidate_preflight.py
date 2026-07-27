#!/usr/bin/env python3
"""Perf-ledger and audit-compile preflight.

Campaign `perf-campaign-20260725`, Meta-Lever #1 institutionalization. Modelled on
frankensqlite's `sql_pipeline_candidate_preflight` (exit 2 = BLOCKED), which is why that
repo sits at a 1.7% void rate after four months while repos that audited once and stopped
sit at 25-91%. Ledger integrity DECAYS; this is the ratchet.

Modes:

1. `--candidate "<lever>" --surface "<target>"` — grep the ledger BEFORE source mutation.
   A prior row on the surface blocks with its retry predicate printed.

2. `--check-new-rows` (default; pre-commit mode) — every NEW `###` REJECT must record
   either an A/A null control or a counted mechanism shown unchanged / missing a
   predeclared numeric mechanism threshold. Every NEW `###` KEEP/WIN/SHIPPED row
   must record both a 64-hex binary SHA-256 and an in-process executing-ELF marker.
   Shell-side hashes do not satisfy the latter.

3. `--check-disk` — print `df -h /data` and block below 120 GiB free.

4. `--run-compile ...` — run one compile command only after the disk guard passes. It
   requires one explicit/shared `CARGO_TARGET_DIR`, preventing per-snapshot target growth.

NOTE: "bit-identical" / "byte-identical" / "0 diffs" are parity proofs, not counted
mechanism refutations, and are deliberately not accepted.

Exit codes: 0 = pass, 2 = BLOCKED, 1 = usage/IO error.
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
LEDGER = REPO / "docs" / "NEGATIVE_EVIDENCE.md"
DEFAULT_DISK_PATH = Path("/data")
DEFAULT_MIN_FREE_GIB = 120.0
DEFAULT_GIT_TIMEOUT_SECONDS = 30.0
DEFAULT_DF_TIMEOUT_SECONDS = 10.0
DEFAULT_COMPILE_TIMEOUT_SECONDS = 3600.0

REJECT_MARK = re.compile(
    r"\bREJECT(?:ED|S)?\b|\bNOSHIP\b|\bNO-SHIP\b|"
    r"\bSLOWER\b|\bLOSS(?:ES)?\b|\bREGRESSION(?:S)?\b|zero-gain|~0-gain",
    re.IGNORECASE,
)
KEEP_MARK = re.compile(r"\bKEEP\b|\bSHIPPED\b|\bWIN\b", re.IGNORECASE)

# --- Campaign policy 2 (2026-07-27): self-speedup vs vs-incumbent -------------
# A self-speedup (our own code before vs after) is MAINTENANCE. A campaign WIN
# requires a ratio against the ACTUAL incumbent, produced by a harness that runs
# the incumbent side by side IN THE SAME INVOCATION. Across 369 campaign commits
# ~60 self-speedups were produced but only 3 new vs-incumbent wins, all from repos
# with a live incumbent arm. Self-speedups may still land and be ledgered -- they
# just must be LABELLED, and never quoted as competitive claims.
#
# A row is admissible if it either declares itself a self-speedup, or shows a
# same-invocation incumbent arm. `SELF_SPEEDUP_LABEL` is the cheap, explicit way
# to comply.
SELF_SPEEDUP_LABEL = re.compile(
    r"\bself-?speedup\b|\bfp-side\b|\bmaintenance\b|\bnot a competitive claim\b|"
    r"\bno incumbent arm\b|\binternal (?:only|speedup)\b",
    re.IGNORECASE,
)
# Evidence that the legacy incumbent actually ran alongside us in this measurement.
INCUMBENT_ARM = re.compile(
    r"(?:pandas|numpy|redis|sqlite|networkx|tantivy|lucene|glibc|whisper\.cpp|"
    r"mermaid-js|scipy|openblas)\s*[0-9]|"
    r"vs[_-]pandas_harness|side[- ]by[- ]side|same invocation|"
    r"incumbent arm|oracle arm|legacy arm",
    re.IGNORECASE,
)
# A competitive-sounding ratio claim, e.g. "3.2x faster than pandas".
COMPETITIVE_RATIO = re.compile(
    r"\d+(?:\.\d+)?\s*[x×]\s*(?:faster|slower|vs\.?|against|over)\b|"
    r"\bvs\.?\s+(?:pandas|numpy|redis|sqlite|networkx|glibc|scipy)\b",
    re.IGNORECASE,
)
ZERO_VERDICT_COUNT = re.compile(
    r"\b(?:0|zero|no)\s+(?:new\s+)?"
    r"(?:REJECT(?:ED|S)?|NOSHIP|NO-SHIP|SLOWER|LOSS(?:ES)?|"
    r"REGRESSION(?:S)?|KEEP(?:S)?|SHIPPED|WIN(?:S)?)\b",
    re.IGNORECASE,
)
NULL_CTRL = re.compile(
    r"(?:null control|null-control|A/A|null arm|null floor|nulls?\s*~\s*1\.0)"
    r"[^\n]{0,160}\d",
    re.IGNORECASE,
)
NEGATED_NULL_CTRL = re.compile(
    r"(?:\bno\b|\bwithout\b|\bmissing\b|\babsent\b|\bnever (?:ran|recorded)\b|"
    r"\bunavailable\b|\bnot (?:run|recorded|measured)\b).{0,40}"
    r"(?:A/A|null control|null-control|null arm|null floor)|"
    r"(?:A/A|null control|null-control|null arm|null floor).{0,40}"
    r"(?:\bmissing\b|\babsent\b|\bunavailable\b|\bN/?A\b|"
    r"\bnot (?:run|recorded|measured)\b)",
    re.IGNORECASE,
)
# Requires a COUNTED quantity shown unchanged. Deliberately excludes bit/byte-identical.
MECHANISM = re.compile(
    r"(?:instructions?|cycles|syscalls?|allocations?|mallocs?|page[- ]faults?|"
    r"branch[- ]miss(?:es)?|cache[- ]miss(?:es)?|IPC)\s*"
    r"(?:count\w*\s*)?(?:are|is|was|were|remained?|stayed?|:)?\s*"
    r"(?:un|not )?chang\w*|identical instruction|same instruction count|"
    r"(?:zero|0)\s+(?:instructions?|cycles|syscalls?|allocations?|mallocs?|"
    r"page[- ]faults?|branch[- ]miss(?:es)?|cache[- ]miss(?:es)?)",
    re.IGNORECASE,
)
# A predeclared numeric COUNTED-mechanism pregate can also decide a row without
# an A/A wall null. Both the measured and required values must be present; vague
# prose such as "cycles missed the target" remains blocked.
COUNTED_THRESHOLD = re.compile(
    r"(?:instructions?|cycles|syscalls?|allocations?|mallocs?|page[- ]faults?|"
    r"branch[- ]miss(?:es)?|cache[- ]miss(?:es)?|IPC)[^\n]{0,160}"
    r"\d[\d_,]*(?:\.\d+)?\s*(?:x|%|cycles?)?[^\n]{0,120}"
    r"(?:required|predeclared|pregate|threshold|limit)[^\n]{0,80}"
    r"\d[\d_,]*(?:\.\d+)?\s*(?:x|%|cycles?)?[^\n]{0,80}"
    r"(?:fail(?:ed)?|miss(?:ed)?|below|short|not (?:meet|clear))",
    re.IGNORECASE,
)
IN_PROCESS_SHA256 = re.compile(
    r"(?:bench_elf_sha256\s*=|running (?:test )?ELF SHA-?256\s*[:=]|"
    r"executing (?:ELF|binary).{0,40}SHA-?256\s*[:=]|"
    r"(?:std::env::)?current_exe.{0,40}(?:self-report(?:ed|ing)?|SHA-?256)\s*[:=]|"
    r"self-report(?:ed|ing)?.{0,40}(?:ELF|binary).{0,40}SHA-?256\s*[:=])\s*"
    r"[0-9a-f]{64}\b",
    re.IGNORECASE,
)
RETRY_MARK = re.compile(
    r"retry predicate|retry condition|retry only|re-?open only|do not retry|"
    r"do not attempt|closing [^.]{0,160} needs|resume:",
    re.IGNORECASE,
)


def _git(*args: str) -> str:
    try:
        result = subprocess.run(
            ["git", *args],
            cwd=REPO,
            capture_output=True,
            text=True,
            check=False,
            timeout=DEFAULT_GIT_TIMEOUT_SECONDS,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise RuntimeError(f"unable to run git {' '.join(args)}: {error}") from error
    if result.returncode:
        detail = result.stderr.strip() or f"exit {result.returncode}"
        raise RuntimeError(f"git {' '.join(args)} failed: {detail}")
    return result.stdout


def added_ledger_sections(base: str, *, cached: bool) -> list[tuple[str, str]]:
    """Return (title, body) for each `###` section added to the ledger vs `base`."""
    args = ["diff"]
    if cached:
        args.append("--cached")
    args.extend(["--unified=0", base, "--", str(LEDGER.relative_to(REPO))])
    diff = _git(*args)
    added = [l[1:] for l in diff.splitlines() if l.startswith("+") and not l.startswith("+++")]
    sections: list[tuple[str, list[str]]] = []
    for line in added:
        if line.startswith("### "):
            sections.append((line[4:].strip(), []))
        elif sections:
            sections[-1][1].append(line)
    return [(t, "\n".join(b)) for t, b in sections]


def validate_new_rows(
    sections: list[tuple[str, str]],
) -> tuple[list[str], list[str], list[str]]:
    blocked_rejects: list[str] = []
    blocked_keeps: list[str] = []
    blocked_unlabelled: list[str] = []
    for title, body in sections:
        blob = f"{title}\n{body}"
        verdict_title = ZERO_VERDICT_COUNT.sub("", title)
        if REJECT_MARK.search(verdict_title) and not (
            has_positive_null_control(blob)
            or MECHANISM.search(blob)
            or COUNTED_THRESHOLD.search(blob)
        ):
            blocked_rejects.append(title)
        if KEEP_MARK.search(verdict_title) and not IN_PROCESS_SHA256.search(blob):
            blocked_keeps.append(title)
        # Policy 2: a row making a competitive ratio claim must either show a
        # same-invocation incumbent arm or declare itself a self-speedup.
        if (
            COMPETITIVE_RATIO.search(blob)
            and not INCUMBENT_ARM.search(blob)
            and not SELF_SPEEDUP_LABEL.search(blob)
        ):
            blocked_unlabelled.append(title)
    return blocked_rejects, blocked_keeps, blocked_unlabelled


def has_positive_null_control(blob: str) -> bool:
    return bool(NULL_CTRL.search(blob) and not NEGATED_NULL_CTRL.search(blob))


def check_new_rows(base: str, *, cached: bool) -> int:
    sections = added_ledger_sections(base, cached=cached)
    if not sections:
        source = "staged" if cached else "working-tree"
        print(f"preflight: no new {source} ledger sections vs {base} — OK")
        return 0

    blocked_rejects, blocked_keeps, blocked_unlabelled = validate_new_rows(sections)
    if blocked_rejects or blocked_keeps or blocked_unlabelled:
        print("preflight: BLOCKED — inadmissible new performance ledger row(s)\n")
        for title in blocked_rejects:
            print(f"  ✗ REJECT without A/A or counted mechanism: {title[:150]}")
        for title in blocked_keeps:
            print(f"  ✗ KEEP without executing-ELF SHA-256: {title[:150]}")
        for title in blocked_unlabelled:
            print(f"  ✗ competitive ratio without an incumbent arm or self-speedup label: {title[:150]}")
        if blocked_unlabelled:
            print(
                "\nCampaign policy 2: a self-speedup (our code before vs after) is "
                "MAINTENANCE, not campaign output.\nA row quoting a competitive ratio "
                "must EITHER:\n"
                "  (a) show a same-invocation incumbent arm (vs_pandas_harness, "
                "'side by side', a versioned incumbent), OR\n"
                "  (b) label itself a self-speedup / fp-side / maintenance.\n"
                "Self-speedups may land and be ledgered — they may never be quoted as "
                "competitive claims."
            )
        if blocked_rejects:
            print(
                "\nEvery REJECT must record ONE of:\n"
                "  (a) an A/A null control\n"
                "  (b) a COUNTED mechanism unchanged, or measured below a "
                "predeclared numeric mechanism threshold — instructions/cycles/"
                "syscalls/allocations/faults\n"
                "'bit-identical' does not satisfy (b): parity is not a mechanism count."
            )
        if blocked_keeps:
            print(
                "\nEvery KEEP/WIN/SHIPPED row must contain:\n"
                "  (a) the 64-hex SHA-256 value\n"
                "  (b) an in-process marker such as bench_elf_sha256/current_exe/"
                "running ELF SHA-256\n"
                "A hash computed by a neighboring shell command does not prove which ELF ran."
            )
        return 2

    print(
        f"preflight: {len(sections)} new ledger section(s); "
        "all REJECT and KEEP rows admissible — OK"
    )
    return 0


def ledger_sections() -> list[tuple[int, str, str]]:
    sections: list[tuple[int, str, list[str]]] = []
    for line_no, line in enumerate(LEDGER.read_text(errors="replace").splitlines(), 1):
        if line.startswith("### "):
            sections.append((line_no, line[4:].strip(), []))
        elif sections:
            sections[-1][2].append(line)
    return [(line_no, title, "\n".join(body)) for line_no, title, body in sections]


def terms_for(value: str) -> list[str]:
    return [term for term in re.split(r"[\s,/:]+", value.strip()) if len(term) > 2]


def contains_surface_term(blob: str, term: str) -> bool:
    """Match a surface token without treating `str` as part of `instrumented`."""
    return bool(
        re.search(
            rf"(?<![A-Za-z0-9]){re.escape(term)}(?![A-Za-z0-9])",
            blob,
            re.IGNORECASE,
        )
    )


def retry_predicate(body: str) -> str:
    paragraphs = re.split(r"\n\s*\n", body)
    for paragraph in paragraphs:
        match = RETRY_MARK.search(paragraph)
        if match:
            predicate = paragraph[match.start() :]
            return re.sub(r"\s+", " ", predicate).strip()[:700]
    for line in body.splitlines():
        match = RETRY_MARK.search(line)
        if match:
            return re.sub(r"\s+", " ", line[match.start() :]).strip()[:700]
    return "NOT_RECORDED"


def check_candidate(candidate: str, surface: str | None) -> int:
    if not LEDGER.is_file():
        print(f"preflight: ledger not found at {LEDGER}", file=sys.stderr)
        return 1

    candidate_terms = terms_for(candidate)
    surface_terms = terms_for(surface or candidate)
    if not candidate_terms or not surface_terms:
        print(
            "preflight: --candidate/--surface need at least one term of 3+ chars",
            file=sys.stderr,
        )
        return 1

    hits: list[tuple[int, str, str]] = []
    for line_no, title, body in ledger_sections():
        if not REJECT_MARK.search(ZERO_VERDICT_COUNT.sub("", title)):
            continue
        blob = f"{title}\n{body}"
        if all(contains_surface_term(blob, term) for term in surface_terms):
            hits.append((line_no, title, retry_predicate(body)))

    if hits:
        print(
            "preflight: BLOCKED — "
            f"{len(hits)} prior REJECT row(s) match target_surface={surface or candidate!r}"
        )
        print(f"proposed_lever={candidate!r}\n")
        for line_no, title, predicate in hits[:10]:
            print(
                f"match decision=REJECT ledger=docs/NEGATIVE_EVIDENCE.md:{line_no}\n"
                f"  title={title[:180]}\n"
                f"  retry_condition={predicate}\n"
            )
        print(
            "Do not mutate source until a printed retry condition is explicitly satisfied. "
            "A missing predicate is itself a ledger defect to repair first."
        )
        return 2

    print(
        "preflight: CLEAR — no prior REJECT matches "
        f"target_surface={surface or candidate!r}; proposed_lever={candidate!r}"
    )
    return 0


def check_disk(path: Path, min_free_gib: float) -> int:
    try:
        report = subprocess.run(
            ["df", "-h", str(path)],
            capture_output=True,
            text=True,
            check=False,
            timeout=DEFAULT_DF_TIMEOUT_SECONDS,
        )
        if report.returncode:
            detail = report.stderr.strip() or f"exit {report.returncode}"
            print(f"disk_guard: df failed for {path}: {detail}", file=sys.stderr)
            return 1
        if report.stdout:
            print(report.stdout.rstrip(), flush=True)
        usage = shutil.disk_usage(path)
    except (OSError, subprocess.TimeoutExpired) as error:
        print(f"disk_guard: unable to inspect {path}: {error}", file=sys.stderr)
        return 1

    free_gib = usage.free / (1024**3)
    if free_gib < min_free_gib:
        print(
            f"disk_guard: BLOCKED — {free_gib:.1f} GiB free at {path}, "
            f"minimum is {min_free_gib:.1f} GiB",
            flush=True,
        )
        return 2
    print(
        f"disk_guard: CLEAR — {free_gib:.1f} GiB free at {path}, "
        f"minimum is {min_free_gib:.1f} GiB",
        flush=True,
    )
    return 0


def run_compile(
    command: list[str],
    *,
    target_dir: str | None,
    disk_path: Path,
    min_free_gib: float,
    timeout_seconds: float,
) -> int:
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        print("preflight: --run-compile requires a command", file=sys.stderr)
        return 1
    tool = Path(command[0]).name
    if tool not in {"cargo", "rustc"}:
        print("preflight: --run-compile accepts cargo or rustc only", file=sys.stderr)
        return 1
    if any("\0" in argument for argument in command):
        print("preflight: compile arguments may not contain NUL bytes", file=sys.stderr)
        return 1
    if timeout_seconds <= 0:
        print("preflight: --compile-timeout-seconds must be positive", file=sys.stderr)
        return 1
    executable = shutil.which(tool)
    if executable is None:
        print(f"preflight: unable to locate {tool} in PATH", file=sys.stderr)
        return 1
    disk_rc = check_disk(disk_path, min_free_gib)
    if disk_rc:
        return disk_rc

    env = os.environ.copy()
    selected_target = target_dir or env.get("CARGO_TARGET_DIR")
    if not selected_target:
        print(
            "preflight: BLOCKED — guarded compiles require one explicit/shared "
            "--target-dir or CARGO_TARGET_DIR",
            file=sys.stderr,
        )
        return 2
    env["CARGO_TARGET_DIR"] = str(Path(selected_target).resolve())
    print(f"compile_guard: target_dir={env['CARGO_TARGET_DIR']}", flush=True)
    print(f"compile_guard: timeout_seconds={timeout_seconds:g}", flush=True)
    safe_command = [executable, *command[1:]]
    try:
        return subprocess.run(
            safe_command,
            cwd=REPO,
            env=env,
            check=False,
            shell=False,
            timeout=timeout_seconds,
        ).returncode
    except subprocess.TimeoutExpired:
        print(
            f"compile_guard: BLOCKED — compile exceeded {timeout_seconds:g}s",
            file=sys.stderr,
        )
        return 2
    except OSError as error:
        print(f"compile_guard: unable to execute {tool}: {error}", file=sys.stderr)
        return 1


def self_test() -> int:
    sha = "a" * 64
    cases = [
        ("reject_without_basis", [("REJECT foo", "bit-identical, 0 diffs")], (1, 0)),
        ("reject_zero_self_only", [("REJECT foo", "target frame 0.000% self")], (1, 0)),
        ("reject_negated_null", [("REJECT foo", "no A/A null control recorded")], (1, 0)),
        ("reject_bare_null", [("REJECT foo", "A/A null control mentioned")], (1, 0)),
        ("reject_perf_stat_only", [("REJECT foo", "perf stat was attempted")], (1, 0)),
        ("reject_claim_only", [("REJECT foo", "no work was removed")], (1, 0)),
        (
            "reject_model_incident_real_loss_without_basis",
            [
                (
                    (
                        "uza04.218 matrix: ONE real loss "
                        "(df_groupby_2strkey_sum @10k 0.86x)"
                    ),
                    (
                        "Four p50 ratios agree; output is bit-identical. "
                        "No executing-ELF identity was recorded."
                    ),
                )
            ],
            (1, 0),
        ),
        ("reject_with_null", [("REJECT foo", "A/A null control 1.001x")], (0, 0)),
        (
            "slower_with_null",
            [("df_groupby_2strkey_sum is SLOWER", "A/A null control 1.001x")],
            (0, 0),
        ),
        (
            "reject_with_count",
            [("REJECT foo", "perf stat instructions unchanged")],
            (0, 0),
        ),
        (
            "reject_with_numeric_counted_pregate",
            [
                (
                    "REJECT packed kernel",
                    (
                        "Counted mechanism: cycles ratio measured 1.191365x versus "
                        "predeclared threshold 1.250000x; it failed the counted pregate."
                    ),
                )
            ],
            (0, 0),
        ),
        (
            "reject_with_vague_counted_pregate",
            [("REJECT packed kernel", "cycles missed the target")],
            (1, 0),
        ),
        ("keep_without_sha", [("KEEP foo", "same worker")], (0, 1)),
        ("keep_shell_sha", [("KEEP foo", f"shell sha256 {sha}")], (0, 1)),
        (
            "keep_unbound_marker",
            [("KEEP foo", f"bench_elf_sha256=unavailable\nshell sha256 {sha}")],
            (0, 1),
        ),
        (
            "keep_executing_sha",
            [("KEEP foo", f"bench_elf_sha256={sha} (123 bytes) /tmp/bench")],
            (0, 0),
        ),
        (
            "summary_zero_rejects_is_not_a_reject",
            [("Integrity summary: 6 commits, 0 REJECTS", "No performance row.")],
            (0, 0),
        ),
        (
            "summary_no_new_keeps_is_not_a_keep",
            [("Audit summary: no new KEEPs", "No performance row.")],
            (0, 0),
        ),
        (
            "summary_zero_losses_is_not_a_reject",
            [("Integrity summary: zero losses", "No performance row.")],
            (0, 0),
        ),
    ]
    failed = []
    for name, sections, expected in cases:
        actual_lists = validate_new_rows(sections)
        actual = (len(actual_lists[0]), len(actual_lists[1]))
        if actual != expected:
            failed.append((name, expected, actual))
    if failed:
        for name, expected, actual in failed:
            print(f"self-test FAIL {name}: expected={expected} actual={actual}")
        return 1
    print(f"preflight self-test: PASS ({len(cases)} cases)")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    modes = ap.add_mutually_exclusive_group()
    modes.add_argument("--candidate", help="Proposed lever description")
    modes.add_argument(
        "--check-new-rows",
        "--check-new-rejects",
        dest="check_new_rows",
        action="store_true",
        help="Validate newly added REJECT and KEEP rows",
    )
    modes.add_argument("--check-disk", action="store_true", help="Run the free-space guard")
    modes.add_argument(
        "--run-compile",
        action="store_true",
        help="Disk-guard and run one cargo/rustc command",
    )
    modes.add_argument("--self-test", action="store_true", help="Run deterministic gate tests")
    ap.add_argument("--surface", help="Target source/workload surface for --candidate")
    ap.add_argument("--base", default="HEAD", help="Git ref to diff the ledger against")
    ap.add_argument(
        "--cached",
        action="store_true",
        help="Inspect staged ledger changes (pre-commit mode)",
    )
    ap.add_argument("--target-dir", help="Shared CARGO_TARGET_DIR for --run-compile")
    ap.add_argument(
        "--compile-timeout-seconds",
        type=float,
        default=DEFAULT_COMPILE_TIMEOUT_SECONDS,
        help="Compile timeout in seconds (default 3600)",
    )
    ap.add_argument(
        "--disk-path",
        type=Path,
        default=DEFAULT_DISK_PATH,
        help="Filesystem path checked by the disk guard (default /data)",
    )
    ap.add_argument(
        "--min-free-gib",
        type=float,
        default=DEFAULT_MIN_FREE_GIB,
        help="Minimum free GiB (default 120)",
    )
    args, command = ap.parse_known_args()
    if command and not args.run_compile:
        ap.error(f"unrecognized arguments: {' '.join(command)}")

    if args.candidate:
        return check_candidate(args.candidate, args.surface)
    if args.check_disk:
        return check_disk(args.disk_path, args.min_free_gib)
    if args.run_compile:
        return run_compile(
            command,
            target_dir=args.target_dir,
            disk_path=args.disk_path,
            min_free_gib=args.min_free_gib,
            timeout_seconds=args.compile_timeout_seconds,
        )
    if args.self_test:
        return self_test()
    return check_new_rows(args.base, cached=args.cached)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as error:
        print(f"preflight: {error}", file=sys.stderr)
        raise SystemExit(1) from None
